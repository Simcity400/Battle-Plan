//! Exact creation-time D3D11 shader reflection for d77/d80 authority.
//!
//! This module reflects the byte-for-byte payload already retained by
//! `RawShaderCreation`. It does not inspect filenames, symbols, bound slots, or
//! a vanilla catalog. The engine program-registration observer supplies the
//! program pointer and entry token; D3DReflect supplies the ordered signature,
//! resource, and constant-buffer descriptors.

#![cfg(windows)]

use crate::{
    RawReflectedConstantBufferVariable, RawReflectedResourceKind,
    RawShaderBytecodeReflectionObservation, RawShaderCreation, RawShaderReflectedResource,
    RawShaderSignatureParameter,
};
use std::ffi::CStr;
use std::mem;
use std::ptr;
use winapi::ctypes::c_void;
use winapi::shared::winerror::SUCCEEDED;
use winapi::um::d3d11shader::{
    ID3D11ShaderReflection, ID3D11ShaderReflectionType, IID_ID3D11ShaderReflection,
    D3D11_SHADER_BUFFER_DESC, D3D11_SHADER_DESC, D3D11_SHADER_INPUT_BIND_DESC,
    D3D11_SHADER_TYPE_DESC, D3D11_SHADER_VARIABLE_DESC, D3D11_SIGNATURE_PARAMETER_DESC,
};
use winapi::um::d3dcommon::{D3D_SIT_CBUFFER, D3D_SIT_SAMPLER, D3D_SIT_TEXTURE};
use winapi::um::d3dcompiler::D3DReflect;

struct Reflection(*mut ID3D11ShaderReflection);

impl Drop for Reflection {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: D3DReflect returned this live COM interface and this
            // wrapper owns exactly one reference.
            unsafe { (*self.0).Release() };
        }
    }
}

/// Reflect one exact shader-creation payload into the neutral live-authority
/// contract. `reflection_sequence` is allocated by the same hook-wide ordered
/// observation clock as the creation/program receipts.
pub fn reflect_shader_creation(
    creation: &RawShaderCreation,
    reflection_sequence: u64,
    shader_program_ptr: usize,
    entry_token: &str,
) -> Result<RawShaderBytecodeReflectionObservation, String> {
    if reflection_sequence == 0
        || shader_program_ptr == 0
        || entry_token.trim().is_empty()
        || creation.bytecode.captured_bytes.is_empty()
    {
        return Err("shader reflection correlation input is incomplete".into());
    }

    let mut raw = ptr::null_mut::<c_void>();
    // SAFETY: the byte slice is live for the call; the output pointer is owned
    // by `Reflection` after a successful HRESULT.
    let hr = unsafe {
        D3DReflect(
            creation.bytecode.captured_bytes.as_ptr().cast(),
            creation.bytecode.captured_bytes.len(),
            &IID_ID3D11ShaderReflection,
            &mut raw,
        )
    };
    if !SUCCEEDED(hr) || raw.is_null() {
        return Err(format!(
            "D3DReflect failed for captured bytecode: HRESULT {hr:#010x}"
        ));
    }
    let reflection = Reflection(raw.cast());
    let mut shader_desc: D3D11_SHADER_DESC = unsafe { mem::zeroed() };
    // SAFETY: `reflection` owns a valid ID3D11ShaderReflection interface.
    let hr = unsafe { (*reflection.0).GetDesc(&mut shader_desc) };
    if !SUCCEEDED(hr) {
        return Err(format!(
            "ID3D11ShaderReflection::GetDesc failed: HRESULT {hr:#010x}"
        ));
    }

    let ordered_inputs = reflect_signatures(reflection.0, shader_desc.InputParameters, true)?;
    let ordered_outputs = reflect_signatures(reflection.0, shader_desc.OutputParameters, false)?;
    let ordered_resources = reflect_resources(reflection.0, shader_desc.BoundResources)?;
    let observed = RawShaderBytecodeReflectionObservation {
        reflection_sequence,
        creation_sequence: creation.creation_sequence,
        shader_ptr: creation.shader_ptr,
        shader_program_ptr,
        stage: creation.stage,
        bytecode_sha256: creation.bytecode.captured_sha256.clone(),
        bytecode_bytes: creation.bytecode.captured_bytes.len() as u64,
        entry_token: entry_token.to_owned(),
        target_profile: creation.target_profile.clone(),
        creator_token: "ID3D11Device::CreateShader+D3DReflect".into(),
        ordered_inputs,
        ordered_outputs,
        ordered_resources,
    };
    observed.validate_against_creation(creation)?;
    Ok(observed)
}

fn reflect_signatures(
    reflection: *mut ID3D11ShaderReflection,
    count: u32,
    input: bool,
) -> Result<Vec<RawShaderSignatureParameter>, String> {
    let mut rows = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut desc: D3D11_SIGNATURE_PARAMETER_DESC = unsafe { mem::zeroed() };
        // SAFETY: the reflection interface is live and `index < count` came
        // from its shader descriptor.
        let hr = unsafe {
            if input {
                (*reflection).GetInputParameterDesc(index, &mut desc)
            } else {
                (*reflection).GetOutputParameterDesc(index, &mut desc)
            }
        };
        if !SUCCEEDED(hr) {
            return Err(format!(
                "shader {} signature descriptor {index} failed: HRESULT {hr:#010x}",
                if input { "input" } else { "output" }
            ));
        }
        rows.push(RawShaderSignatureParameter {
            semantic_token: required_c_string(desc.SemanticName, "signature semantic")?,
            semantic_index: desc.SemanticIndex,
            register_index: desc.Register,
            system_value_type_u32: desc.SystemValueType as u32,
            component_type_u32: desc.ComponentType as u32,
            mask_u8: desc.Mask,
            read_write_mask_u8: desc.ReadWriteMask,
            stream_index: desc.Stream,
            min_precision_u32: desc.MinPrecision as u32,
        });
    }
    Ok(rows)
}

fn reflect_resources(
    reflection: *mut ID3D11ShaderReflection,
    count: u32,
) -> Result<Vec<RawShaderReflectedResource>, String> {
    let mut rows = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut desc: D3D11_SHADER_INPUT_BIND_DESC = unsafe { mem::zeroed() };
        // SAFETY: the reflection interface is live and `index < count` came
        // from its shader descriptor.
        let hr = unsafe { (*reflection).GetResourceBindingDesc(index, &mut desc) };
        if !SUCCEEDED(hr) {
            return Err(format!(
                "shader resource descriptor {index} failed: HRESULT {hr:#010x}"
            ));
        }
        let semantic_token = required_c_string(desc.Name, "resource name")?;
        let kind = match desc.Type {
            D3D_SIT_TEXTURE => RawReflectedResourceKind::Texture,
            D3D_SIT_SAMPLER => RawReflectedResourceKind::Sampler,
            D3D_SIT_CBUFFER => RawReflectedResourceKind::ConstantBuffer,
            value => RawReflectedResourceKind::Other(value as u32),
        };
        let (constant_buffer_byte_count, constant_buffer_layout_token, ordered_variables) =
            if kind == RawReflectedResourceKind::ConstantBuffer {
                reflect_constant_buffer(reflection, &semantic_token)?
            } else {
                (None, None, Vec::new())
            };
        rows.push(RawShaderReflectedResource {
            semantic_token,
            kind,
            bind_point: desc.BindPoint,
            bind_count: desc.BindCount,
            flags_u32: desc.uFlags,
            return_type_u32: desc.ReturnType as u32,
            dimension_u32: desc.Dimension as u32,
            sample_count: desc.NumSamples,
            constant_buffer_byte_count,
            constant_buffer_layout_token,
            ordered_variables,
        });
    }
    Ok(rows)
}

fn reflect_constant_buffer(
    reflection: *mut ID3D11ShaderReflection,
    resource_name: &str,
) -> Result<
    (
        Option<u32>,
        Option<String>,
        Vec<RawReflectedConstantBufferVariable>,
    ),
    String,
> {
    let name = std::ffi::CString::new(resource_name)
        .map_err(|_| "reflected constant-buffer name contains NUL".to_owned())?;
    // SAFETY: reflection is live and the CString is NUL-terminated.
    let buffer = unsafe { (*reflection).GetConstantBufferByName(name.as_ptr()) };
    if buffer.is_null() {
        return Err("reflected constant buffer has no matching descriptor".into());
    }
    let mut desc: D3D11_SHADER_BUFFER_DESC = unsafe { mem::zeroed() };
    // SAFETY: the reflection API returned this constant-buffer interface.
    let hr = unsafe { (*buffer).GetDesc(&mut desc) };
    if !SUCCEEDED(hr) {
        return Err(format!(
            "constant-buffer GetDesc failed: HRESULT {hr:#010x}"
        ));
    }
    let reflected_name = required_c_string(desc.Name, "constant-buffer name")?;
    if reflected_name != resource_name {
        return Err("constant-buffer resource and descriptor names differ".into());
    }
    let mut variables = Vec::with_capacity(desc.Variables as usize);
    for index in 0..desc.Variables {
        // SAFETY: `index < Variables` came from this buffer descriptor.
        let variable = unsafe { (*buffer).GetVariableByIndex(index) };
        if variable.is_null() {
            return Err("constant-buffer variable interface is null".into());
        }
        let mut variable_desc: D3D11_SHADER_VARIABLE_DESC = unsafe { mem::zeroed() };
        // SAFETY: variable is a live reflection interface.
        let hr = unsafe { (*variable).GetDesc(&mut variable_desc) };
        if !SUCCEEDED(hr) {
            return Err(format!(
                "constant-buffer variable {index} GetDesc failed: HRESULT {hr:#010x}"
            ));
        }
        // SAFETY: variable is a live reflection interface.
        let ty = unsafe { (*variable).GetType() };
        if ty.is_null() {
            return Err("constant-buffer variable type is null".into());
        }
        variables.push(RawReflectedConstantBufferVariable {
            name: required_c_string(variable_desc.Name, "constant-buffer variable name")?,
            start_offset: variable_desc.StartOffset,
            byte_count: variable_desc.Size,
            flags_u32: variable_desc.uFlags,
            type_token: reflect_type_token(ty, 0)?,
        });
    }
    let mut layout = String::new();
    layout.push_str("d3d11-cbuffer-v1[");
    append_text(&mut layout, &reflected_name);
    layout.push_str(&format!(
        ";{};{};{}",
        desc.Type as u32, desc.Size, desc.uFlags
    ));
    for variable in &variables {
        layout.push(';');
        append_text(&mut layout, &variable.name);
        layout.push_str(&format!(
            ":{}:{}:{}:",
            variable.start_offset, variable.byte_count, variable.flags_u32
        ));
        append_text(&mut layout, &variable.type_token);
    }
    layout.push(']');
    Ok((Some(desc.Size), Some(layout), variables))
}

fn reflect_type_token(ty: *mut ID3D11ShaderReflectionType, depth: u32) -> Result<String, String> {
    if depth >= 256 {
        return Err("shader-reflection type nesting exceeds the fail-closed bound".into());
    }
    let mut desc: D3D11_SHADER_TYPE_DESC = unsafe { mem::zeroed() };
    // SAFETY: ty is a live reflection type interface.
    let hr = unsafe { (*ty).GetDesc(&mut desc) };
    if !SUCCEEDED(hr) {
        return Err(format!(
            "shader-reflection type GetDesc failed: HRESULT {hr:#010x}"
        ));
    }
    let mut token = format!(
        "d3d11-type-v1[{};{};{};{};{};{};{};",
        desc.Class as u32,
        desc.Type as u32,
        desc.Rows,
        desc.Columns,
        desc.Elements,
        desc.Members,
        desc.Offset
    );
    append_optional_text(&mut token, desc.Name)?;
    for index in 0..desc.Members {
        // SAFETY: `index < Members` came from this type descriptor.
        let member = unsafe { (*ty).GetMemberTypeByIndex(index) };
        if member.is_null() {
            return Err("shader-reflection member type is null".into());
        }
        // SAFETY: `index < Members` came from this type descriptor.
        let member_name = unsafe { (*ty).GetMemberTypeName(index) };
        token.push(';');
        append_optional_text(&mut token, member_name)?;
        token.push(':');
        let nested = reflect_type_token(member, depth + 1)?;
        append_text(&mut token, &nested);
    }
    token.push(']');
    Ok(token)
}

fn required_c_string(value: *const i8, label: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("reflected {label} is null"));
    }
    // SAFETY: D3DReflect descriptor string pointers are NUL-terminated and
    // remain valid for the lifetime of the reflection interface.
    let value = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|_| format!("reflected {label} is not UTF-8"))?
        .to_owned();
    if value.trim().is_empty() {
        return Err(format!("reflected {label} is blank"));
    }
    Ok(value)
}

fn append_optional_text(out: &mut String, value: *const i8) -> Result<(), String> {
    if value.is_null() {
        out.push_str("0:");
        return Ok(());
    }
    // SAFETY: D3DReflect descriptor string pointers are NUL-terminated and
    // remain valid for the lifetime of the reflection interface.
    let value = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|_| "reflected type/member name is not UTF-8".to_owned())?;
    append_text(out, value);
    Ok(())
}

fn append_text(out: &mut String, value: &str) {
    out.push_str(&value.len().to_string());
    out.push(':');
    out.push_str(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RawPayloadSnapshot, RawPayloadSource, RawShaderCompilerInvocation, RawShaderStage,
    };
    use sha2::{Digest, Sha256};
    use winapi::um::d3dcommon::{ID3DBlob, D3D_SHADER_MACRO};
    use winapi::um::d3dcompiler::D3DCompile;

    struct Blob(*mut ID3DBlob);
    impl Drop for Blob {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { (*self.0).Release() };
            }
        }
    }

    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(DIGITS[(byte >> 4) as usize] as char);
            out.push(DIGITS[(byte & 15) as usize] as char);
        }
        out
    }

    #[test]
    fn compiled_shader_reflection_preserves_exact_order_and_cbuffer_layout() {
        let source = br#"
cbuffer FrameData : register(b1) {
    float4 tint;
    row_major float4x4 transform;
};
Texture2D Diffuse : register(t3);
SamplerState LinearSampler : register(s2);
float4 PSMain(float2 uv : TEXCOORD0) : SV_Target0 {
    return Diffuse.Sample(LinearSampler, uv) * tint;
}
"#;
        let mut code = ptr::null_mut::<ID3DBlob>();
        let mut errors = ptr::null_mut::<ID3DBlob>();
        let no_defines = ptr::null::<D3D_SHADER_MACRO>();
        let hr = unsafe {
            D3DCompile(
                source.as_ptr().cast(),
                source.len(),
                b"reflection-test.hlsl\0".as_ptr().cast(),
                no_defines,
                ptr::null_mut(),
                b"PSMain\0".as_ptr().cast(),
                b"ps_5_0\0".as_ptr().cast(),
                0,
                0,
                &mut code,
                &mut errors,
            )
        };
        let _errors = Blob(errors);
        assert!(SUCCEEDED(hr), "D3DCompile failed: {hr:#010x}");
        let code = Blob(code);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (*code.0).GetBufferPointer().cast::<u8>(),
                (*code.0).GetBufferSize(),
            )
        }
        .to_vec();
        let digest = Sha256::digest(&bytes);
        let creation = RawShaderCreation {
            creation_sequence: 4,
            shader_ptr: 0x1000,
            stage: RawShaderStage::Pixel,
            compiler_identity: "D3DCompile".into(),
            target_profile: "ps_5_0".into(),
            compiler_flags_u64: 0,
            compiler_invocation: Some(RawShaderCompilerInvocation {
                source_name: "reflection-test.hlsl".into(),
                source_payload_sha256: hex(&Sha256::digest(source)),
                source_payload_bytes: source.len() as u64,
                entry_token: "PSMain".into(),
                ordered_defines: Vec::new(),
            }),
            bytecode: RawPayloadSnapshot {
                captured_bytes: bytes,
                captured_sha256: hex(&digest),
                source: RawPayloadSource::RuntimeGenerated {
                    reason: "D3DCompile output".into(),
                },
            },
        };
        let observed = reflect_shader_creation(&creation, 5, 0x2000, "PSMain").unwrap();
        assert_eq!(observed.ordered_inputs.len(), 1);
        assert_eq!(observed.ordered_inputs[0].semantic_token, "TEXCOORD");
        assert_eq!(observed.ordered_outputs.len(), 1);
        assert_eq!(observed.ordered_outputs[0].semantic_token, "SV_Target");
        assert_eq!(observed.ordered_resources.len(), 3);
        let cbuffer = observed
            .ordered_resources
            .iter()
            .find(|row| row.kind == RawReflectedResourceKind::ConstantBuffer)
            .unwrap();
        assert_eq!(cbuffer.semantic_token, "FrameData");
        assert_eq!(cbuffer.bind_point, 1);
        assert_eq!(cbuffer.ordered_variables.len(), 2);
        assert_eq!(cbuffer.ordered_variables[0].name, "tint");
        assert_eq!(cbuffer.ordered_variables[1].name, "transform");
        assert!(cbuffer
            .constant_buffer_layout_token
            .as_deref()
            .unwrap()
            .starts_with("d3d11-cbuffer-v1["));
        observed.validate_against_creation(&creation).unwrap();
    }
}
