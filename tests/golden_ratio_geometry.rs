use gpui::{phi, px, rems, AbsoluteLength, DefiniteLength};

const PHI: DefiniteLength = phi();

#[test]
fn phi_keeps_its_const_fraction_and_original_float_value() {
    let DefiniteLength::Fraction(value) = PHI else {
        panic!("phi must remain a parent-relative fraction");
    };
    // This is the exact f32 representation of the former decimal constant.
    assert_eq!(value.to_bits(), 0x3fcf_1bbd);
    let mathematical_phi = ((1.0 + 5.0_f64.sqrt()) / 2.0) as f32;
    assert_eq!(value, mathematical_phi);
    assert!((value * value - value - 1.0).abs() <= 2.0 * f32::EPSILON);
}

#[test]
fn phi_resolves_pixel_and_rem_bases_without_changing_geometry() {
    let original = f32::from_bits(0x3fcf_1bbd);
    for rem_size in [8.0, 16.0, 32.0] {
        for base in [-640.0, -1.0, 0.0, 1.0, 100.0, 1_048_576.0] {
            let pixels = PHI.to_pixels(AbsoluteLength::Pixels(px(base)), px(rem_size));
            assert_eq!(pixels, px(base * original));
            let rem_pixels = PHI.to_pixels(AbsoluteLength::Rems(rems(base)), px(rem_size));
            assert_eq!(rem_pixels, px(base * rem_size * original));
        }
    }
}
