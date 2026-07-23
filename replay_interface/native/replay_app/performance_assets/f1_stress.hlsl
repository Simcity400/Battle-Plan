struct VsIn {
    float2 position : POSITION;
};

struct VsOut {
    float4 position : SV_Position;
};

VsOut vs_main(VsIn input) {
    VsOut output;
    output.position = float4(input.position, 0.0f, 1.0f);
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    float2 cell = frac(input.position.xy * float2(0.03125f, 0.046875f));
    return float4(cell.x, cell.y, 0.375f, 1.0f);
}
