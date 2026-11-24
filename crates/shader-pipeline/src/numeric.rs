use std::collections::{BTreeMap, BTreeSet};

pub fn d3d_intrinsics(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !bytes.len().is_multiple_of(4) || bytes.len() < 20 {
        return Err("truncated SPIR-V numeric input".into());
    }
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        if count == 0 {
            return Err("empty SPIR-V instruction".into());
        }
        let instruction = words
            .get(at..at + count)
            .ok_or("truncated SPIR-V instruction")?;
        instructions.push(instruction);
        at += count;
    }
    let mut glsl = Vec::new();
    let mut floats = BTreeMap::new();
    let mut patch = Patch {
        next: words[3],
        declarations: Vec::new(),
        uints: BTreeMap::new(),
        bools: BTreeMap::new(),
        constants: BTreeMap::new(),
        non_nan: BTreeSet::new(),
        float_types: BTreeMap::new(),
        float_constants: BTreeMap::new(),
        bool_constants: BTreeMap::new(),
    };
    for instruction in &instructions {
        match (instruction[0] & 0xffff, &instruction[1..]) {
            (11, [id, name @ ..]) => {
                let name: Vec<u8> = name
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .take_while(|byte| *byte != 0)
                    .collect();
                if name == b"GLSL.std.450" {
                    glsl.push(*id);
                }
            }
            (20, [id]) => {
                patch.bools.insert(1, *id);
            }
            (21, [id, 32, 0]) => {
                patch.uints.insert(1, *id);
            }
            (22, [id, width]) => {
                floats.insert(*id, (*width, 1));
            }
            (23, [id, component, count]) => {
                if let Some(&(width, _)) = floats.get(component) {
                    floats.insert(*id, (width, *count));
                }
                if patch.uints.get(&1) == Some(component) {
                    patch.uints.insert(*count, *id);
                }
                if patch.bools.get(&1) == Some(component) {
                    patch.bools.insert(*count, *id);
                }
            }
            (43, [ty, id, bits])
                if floats.get(ty) == Some(&(32, 1)) && bits & 0x7fff_ffff <= 0x7f80_0000 =>
            {
                patch.non_nan.insert(*id);
            }
            (44, [ty, id, components @ ..])
                if floats.contains_key(ty)
                    && components.iter().all(|id| patch.non_nan.contains(id)) =>
            {
                patch.non_nan.insert(*id);
            }
            _ => {}
        }
    }
    for (&ty, &(width, count)) in &floats {
        if width == 32 {
            patch.float_types.insert(count, ty);
        }
    }
    let values: BTreeMap<u32, u32> = instructions
        .iter()
        .filter(|instruction| {
            let opcode = instruction[0] & 0xffff;
            matches!(opcode, 1 | 12 | 41..) && !matches!(opcode, 71..=75 | 332 | 5632..=5633)
        })
        .filter_map(|instruction| match &instruction[1..] {
            [ty, id, ..] if floats.contains_key(ty) => Some((*id, *ty)),
            _ => None,
        })
        .collect();
    let mut body = Vec::new();
    let mut declarations_at = None;
    for instruction in instructions {
        if instruction[0] & 0xffff == 54 && declarations_at.is_none() {
            declarations_at = Some(body.len());
        }
        if let (12, [ty, result, set, operation, arguments @ ..]) =
            (instruction[0] & 0xffff, &instruction[1..])
            && glsl.contains(set)
            && matches!(operation, 37 | 40 | 43 | 48 | 49 | 79 | 80 | 81)
        {
            let &(width, count) = floats
                .get(ty)
                .ok_or("floating SPIR-V intrinsic has a nonfloating result")?;
            if width != 32 {
                return Err(format!(
                    "D3D numeric intrinsic on {width}-bit floating values is not reproduced"
                ));
            }
            match (*operation, arguments) {
                (37 | 79, [left, right]) => {
                    patch.extremum(&mut body, *set, 37, *ty, count, *result, *left, *right)
                }
                (40 | 80, [left, right]) => {
                    patch.extremum(&mut body, *set, 40, *ty, count, *result, *left, *right)
                }
                (43 | 81, [value, low, high]) => {
                    let middle = patch.id();
                    patch.extremum(&mut body, *set, 40, *ty, count, middle, *value, *low);
                    patch.extremum(&mut body, *set, 37, *ty, count, *result, middle, *high);
                }
                (48, [edge, value]) => patch.step(&mut body, *ty, count, *result, *edge, *value),
                (49, [low, high, value]) => {
                    patch.smoothstep(&mut body, *set, *ty, count, *result, *low, *high, *value)
                }
                _ => return Err("invalid floating SPIR-V intrinsic arity".into()),
            }
        } else if let (operation @ (156 | 157), [ty, result, value]) =
            (instruction[0] & 0xffff, &instruction[1..])
        {
            let &(width, count) = values
                .get(value)
                .and_then(|ty| floats.get(ty))
                .ok_or("floating SPIR-V classification operand has no floating type")?;
            if width != 32 {
                return Err(format!(
                    "D3D classification of {width}-bit floating values is not reproduced"
                ));
            }
            let classification = patch.classify(&mut body, count, *value, operation == 156);
            instruction_copy(&mut body, *ty, *result, classification);
        } else if let (operation @ 180..=191, [ty, result, left, right]) =
            (instruction[0] & 0xffff, &instruction[1..])
        {
            let left_type = values
                .get(left)
                .ok_or("floating SPIR-V comparison operand has no floating type")?;
            if values.get(right) != Some(left_type) {
                return Err("floating SPIR-V comparison operand types differ".into());
            }
            let &(width, count) = &floats[left_type];
            if width != 32 {
                return Err(format!(
                    "D3D comparison of {width}-bit floating values is not reproduced"
                ));
            }
            patch.compare(&mut body, operation, *ty, count, *result, *left, *right);
        } else {
            body.extend_from_slice(instruction);
        }
    }
    let split = declarations_at.unwrap_or(body.len());
    let mut out = words[..5].to_vec();
    out[3] = patch.next;
    out.extend_from_slice(&body[..split]);
    out.extend(patch.declarations);
    out.extend_from_slice(&body[split..]);
    Ok(out.into_iter().flat_map(u32::to_le_bytes).collect())
}

struct Patch {
    next: u32,
    declarations: Vec<u32>,
    uints: BTreeMap<u32, u32>,
    bools: BTreeMap<u32, u32>,
    constants: BTreeMap<(u32, u32), u32>,
    non_nan: BTreeSet<u32>,
    float_types: BTreeMap<u32, u32>,
    float_constants: BTreeMap<(u32, u32), u32>,
    bool_constants: BTreeMap<(u32, bool), u32>,
}

fn instruction(out: &mut Vec<u32>, opcode: u32, operands: &[u32]) {
    out.push(((operands.len() as u32 + 1) << 16) | opcode);
    out.extend_from_slice(operands);
}

fn instruction_copy(out: &mut Vec<u32>, ty: u32, result: u32, value: u32) {
    instruction(out, 83, &[ty, result, value]);
}

impl Patch {
    fn bool_constant(&mut self, count: u32, value: bool) -> u32 {
        if let Some(&id) = self.bool_constants.get(&(count, value)) {
            return id;
        }
        let (_, ty) = self.types(count);
        let scalar = (count != 1).then(|| self.bool_constant(1, value));
        let id = self.id();
        if let Some(scalar) = scalar {
            let mut operands = vec![ty, id];
            operands.extend(std::iter::repeat_n(scalar, count as usize));
            instruction(&mut self.declarations, 44, &operands);
        } else {
            instruction(
                &mut self.declarations,
                if value { 41 } else { 42 },
                &[ty, id],
            );
        }
        self.bool_constants.insert((count, value), id);
        id
    }

    fn compare(
        &mut self,
        out: &mut Vec<u32>,
        operation: u32,
        ty: u32,
        count: u32,
        result: u32,
        left: u32,
        right: u32,
    ) {
        let left_known = self.non_nan.contains(&left);
        let right_known = self.non_nan.contains(&right);
        if left_known && right_known {
            instruction(out, operation, &[ty, result, left, right]);
            return;
        }
        let numeric = self.id();
        instruction(out, operation & !1, &[ty, numeric, left, right]);
        let on_nan = self.bool_constant(count, operation & 1 != 0);
        let selected = if right_known {
            numeric
        } else {
            let right_nan = self.nan(out, count, right);
            let selected = if left_known { result } else { self.id() };
            instruction(out, 169, &[ty, selected, right_nan, on_nan, numeric]);
            selected
        };
        if !left_known {
            let left_nan = self.nan(out, count, left);
            instruction(out, 169, &[ty, result, left_nan, on_nan, selected]);
        }
    }

    fn float_constant(&mut self, count: u32, value: f32) -> u32 {
        let bits = value.to_bits();
        if let Some(&id) = self.float_constants.get(&(count, bits)) {
            return id;
        }
        let ty = self.float_types[&count];
        let scalar = if count == 1 {
            None
        } else {
            Some(self.float_constant(1, value))
        };
        let id = self.id();
        if let Some(scalar) = scalar {
            let mut operands = vec![ty, id];
            operands.extend(std::iter::repeat_n(scalar, count as usize));
            instruction(&mut self.declarations, 44, &operands);
        } else {
            instruction(&mut self.declarations, 43, &[ty, id, bits]);
        }
        self.float_constants.insert((count, bits), id);
        if !value.is_nan() {
            self.non_nan.insert(id);
        }
        id
    }

    fn step(
        &mut self,
        out: &mut Vec<u32>,
        ty: u32,
        count: u32,
        result: u32,
        edge: u32,
        value: u32,
    ) {
        let (_, boolean) = self.types(count);
        let zero = self.float_constant(count, 0.0);
        let one = self.float_constant(count, 1.0);
        let ordered = self.id();
        instruction(out, 190, &[boolean, ordered, value, edge]);
        let numeric = self.id();
        instruction(out, 169, &[ty, numeric, ordered, one, zero]);
        let value_nan = self.nan(out, count, value);
        let edge_nan = self.nan(out, count, edge);
        let value_selected = self.id();
        instruction(out, 169, &[ty, value_selected, value_nan, zero, numeric]);
        instruction(out, 169, &[ty, result, edge_nan, zero, value_selected]);
        self.non_nan.insert(result);
    }

    fn smoothstep(
        &mut self,
        out: &mut Vec<u32>,
        set: u32,
        ty: u32,
        count: u32,
        result: u32,
        low: u32,
        high: u32,
        value: u32,
    ) {
        let zero = self.float_constant(count, 0.0);
        let one = self.float_constant(count, 1.0);
        let two = self.float_constant(count, 2.0);
        let three = self.float_constant(count, 3.0);
        let span = self.id();
        let offset = self.id();
        let reciprocal = self.id();
        let ratio = self.id();
        instruction(out, 131, &[ty, span, high, low]);
        instruction(out, 131, &[ty, offset, value, low]);
        instruction(out, 136, &[ty, reciprocal, one, span]);
        instruction(out, 133, &[ty, ratio, reciprocal, offset]);
        let positive = self.id();
        let bounded = self.id();
        self.extremum(out, set, 40, ty, count, positive, ratio, zero);
        self.extremum(out, set, 37, ty, count, bounded, positive, one);
        let scaled = self.id();
        let complement = self.id();
        let squared = self.id();
        instruction(out, 133, &[ty, scaled, bounded, two]);
        instruction(out, 131, &[ty, complement, three, scaled]);
        instruction(out, 133, &[ty, squared, bounded, bounded]);
        instruction(out, 133, &[ty, result, squared, complement]);
        self.non_nan.insert(result);
    }

    fn id(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }

    fn types(&mut self, count: u32) -> (u32, u32) {
        let scalar_uint = match self.uints.get(&1) {
            Some(&id) => id,
            None => {
                let id = self.id();
                instruction(&mut self.declarations, 21, &[id, 32, 0]);
                self.uints.insert(1, id);
                id
            }
        };
        let scalar_bool = match self.bools.get(&1) {
            Some(&id) => id,
            None => {
                let id = self.id();
                instruction(&mut self.declarations, 20, &[id]);
                self.bools.insert(1, id);
                id
            }
        };
        let uint = match self.uints.get(&count) {
            Some(&id) => id,
            None => {
                let id = self.id();
                instruction(&mut self.declarations, 23, &[id, scalar_uint, count]);
                self.uints.insert(count, id);
                id
            }
        };
        let boolean = match self.bools.get(&count) {
            Some(&id) => id,
            None => {
                let id = self.id();
                instruction(&mut self.declarations, 23, &[id, scalar_bool, count]);
                self.bools.insert(count, id);
                id
            }
        };
        (uint, boolean)
    }

    fn constant(&mut self, count: u32, value: u32) -> u32 {
        if let Some(&id) = self.constants.get(&(count, value)) {
            return id;
        }
        let (ty, _) = self.types(count);
        let scalar = if count == 1 {
            None
        } else {
            Some(self.constant(1, value))
        };
        let id = self.id();
        if let Some(scalar) = scalar {
            let mut operands = vec![ty, id];
            operands.extend(std::iter::repeat_n(scalar, count as usize));
            instruction(&mut self.declarations, 44, &operands);
        } else {
            instruction(&mut self.declarations, 43, &[ty, id, value]);
        }
        self.constants.insert((count, value), id);
        id
    }

    fn nan(&mut self, out: &mut Vec<u32>, count: u32, value: u32) -> u32 {
        self.classify(out, count, value, true)
    }

    fn classify(&mut self, out: &mut Vec<u32>, count: u32, value: u32, nan: bool) -> u32 {
        let (uint, boolean) = self.types(count);
        let mask = self.constant(count, 0x7fff_ffff);
        let infinity = self.constant(count, 0x7f80_0000);
        let bits = self.id();
        let magnitude = self.id();
        let result = self.id();
        instruction(out, 124, &[uint, bits, value]);
        instruction(out, 199, &[uint, magnitude, bits, mask]);
        instruction(
            out,
            if nan { 172 } else { 170 },
            &[boolean, result, magnitude, infinity],
        );
        result
    }

    fn extremum(
        &mut self,
        out: &mut Vec<u32>,
        set: u32,
        operation: u32,
        ty: u32,
        count: u32,
        result: u32,
        left: u32,
        right: u32,
    ) {
        let (left_known, right_known) =
            (self.non_nan.contains(&left), self.non_nan.contains(&right));
        if left_known || right_known {
            self.non_nan.insert(result);
        }
        if left_known && right_known {
            instruction(out, 12, &[ty, result, set, operation, left, right]);
            return;
        }
        let finite = self.id();
        instruction(out, 12, &[ty, finite, set, operation, left, right]);
        let selected = if right_known {
            finite
        } else {
            let right_nan = self.nan(out, count, right);
            let selected = if left_known { result } else { self.id() };
            instruction(out, 169, &[ty, selected, right_nan, left, finite]);
            selected
        };
        if !left_known {
            let left_nan = self.nan(out, count, left);
            instruction(out, 169, &[ty, result, left_nan, right, selected]);
        }
    }
}
