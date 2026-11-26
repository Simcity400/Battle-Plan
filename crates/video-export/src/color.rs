pub fn rgba_to_nv12_bt709(rgba: &[u8], width: usize, height: usize, out: &mut Vec<u8>) {
    debug_assert_eq!(width % 2, 0);
    debug_assert_eq!(height % 2, 0);
    debug_assert_eq!(rgba.len(), width * height * 4);
    let y_len = width * height;
    let uv_len = y_len / 2;
    out.resize(y_len + uv_len, 0);
    let (y_plane, uv_plane) = out.split_at_mut(y_len);
    for row in 0..height {
        let src_row = &rgba[row * width * 4..(row + 1) * width * 4];
        let dst_row = &mut y_plane[row * width..(row + 1) * width];
        for col in 0..width {
            let p = col * 4;
            let r = i32::from(src_row[p]);
            let g = i32::from(src_row[p + 1]);
            let b = i32::from(src_row[p + 2]);
            let y = 16 + ((11_966 * r + 40_254 * g + 4_064 * b + 32_768) >> 16);
            dst_row[col] = y.clamp(16, 235) as u8;
        }
    }
    for row_pair in 0..(height / 2) {
        let top = &rgba[(row_pair * 2) * width * 4..(row_pair * 2 + 1) * width * 4];
        let bot = &rgba[(row_pair * 2 + 1) * width * 4..(row_pair * 2 + 2) * width * 4];
        let uv_row = &mut uv_plane[row_pair * width..(row_pair + 1) * width];
        for col_pair in 0..(width / 2) {
            let p0 = col_pair * 8;
            let p1 = p0 + 4;
            let r = (i32::from(top[p0]) + i32::from(top[p1]) + i32::from(bot[p0]) + i32::from(bot[p1]) + 2) >> 2;
            let g = (i32::from(top[p0 + 1]) + i32::from(top[p1 + 1]) + i32::from(bot[p0 + 1]) + i32::from(bot[p1 + 1]) + 2) >> 2;
            let b = (i32::from(top[p0 + 2]) + i32::from(top[p1 + 2]) + i32::from(bot[p0 + 2]) + i32::from(bot[p1 + 2]) + 2) >> 2;
            let cb = 128 + ((-6_596 * r - 22_179 * g + 28_775 * b + 32_768) >> 16);
            let cr = 128 + ((28_775 * r - 26_149 * g - 2_626 * b + 32_768) >> 16);
            uv_row[col_pair * 2] = cb.clamp(16, 240) as u8;
            uv_row[col_pair * 2 + 1] = cr.clamp(16, 240) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rgba_to_nv12_bt709;

    fn solid_nv12(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let rgba = [r, g, b, 255].repeat(4);
        let mut out = Vec::new();
        rgba_to_nv12_bt709(&rgba, 2, 2, &mut out);
        assert_eq!(out.len(), 6);
        (out[0], out[4], out[5])
    }

    #[test]
    fn bt709_limited_range_primaries_match_reference_values() {
        assert_eq!(solid_nv12(0, 0, 0), (16, 128, 128));
        assert_eq!(solid_nv12(255, 255, 255), (235, 128, 128));
        let (ry, rcb, rcr) = solid_nv12(255, 0, 0);
        assert!((i16::from(ry) - 63).abs() <= 1, "red Y={ry}");
        assert!((i16::from(rcb) - 102).abs() <= 1, "red Cb={rcb}");
        assert!((i16::from(rcr) - 240).abs() <= 1, "red Cr={rcr}");
        let (gy, gcb, gcr) = solid_nv12(0, 255, 0);
        assert!((i16::from(gy) - 173).abs() <= 1, "green Y={gy}");
        assert!((i16::from(gcb) - 42).abs() <= 1, "green Cb={gcb}");
        assert!((i16::from(gcr) - 26).abs() <= 1, "green Cr={gcr}");
        let (by, bcb, bcr) = solid_nv12(0, 0, 255);
        assert!((i16::from(by) - 32).abs() <= 1, "blue Y={by}");
        assert!((i16::from(bcb) - 240).abs() <= 1, "blue Cb={bcb}");
        assert!((i16::from(bcr) - 118).abs() <= 1, "blue Cr={bcr}");
    }
}
