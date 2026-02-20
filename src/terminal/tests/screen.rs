use crate::terminal::screen::{hsl_to_rgb, hsv_to_rgb, rgb_to_hsl, rgb_to_hsv, ColorHSL, ColorHSV};

// ─── Color Math ────────────────────────────────────────────────────────────

#[test]
fn test_rgb_to_hsv_red() {
    let hsv = rgb_to_hsv(255, 0, 0);
    assert!(
        (hsv.h - 0.0).abs() < 1.0,
        "hue should be ~0 for red, got {}",
        hsv.h
    );
    assert!(
        (hsv.s - 1.0).abs() < 0.01,
        "saturation should be 1.0, got {}",
        hsv.s
    );
    assert!(
        (hsv.v - 1.0).abs() < 0.01,
        "value should be 1.0, got {}",
        hsv.v
    );
}

#[test]
fn test_rgb_to_hsv_green() {
    let hsv = rgb_to_hsv(0, 255, 0);
    assert!(
        (hsv.h - 120.0).abs() < 1.0,
        "hue should be ~120 for green, got {}",
        hsv.h
    );
    assert!((hsv.s - 1.0).abs() < 0.01);
    assert!((hsv.v - 1.0).abs() < 0.01);
}

#[test]
fn test_rgb_to_hsv_blue() {
    let hsv = rgb_to_hsv(0, 0, 255);
    assert!(
        (hsv.h - 240.0).abs() < 1.0,
        "hue should be ~240 for blue, got {}",
        hsv.h
    );
    assert!((hsv.s - 1.0).abs() < 0.01);
    assert!((hsv.v - 1.0).abs() < 0.01);
}

#[test]
fn test_rgb_to_hsv_black() {
    let hsv = rgb_to_hsv(0, 0, 0);
    assert!(
        (hsv.s - 0.0).abs() < 0.01,
        "saturation of black should be 0"
    );
    assert!((hsv.v - 0.0).abs() < 0.01, "value of black should be 0");
}

#[test]
fn test_rgb_to_hsv_white() {
    let hsv = rgb_to_hsv(255, 255, 255);
    assert!(
        (hsv.s - 0.0).abs() < 0.01,
        "saturation of white should be 0"
    );
    assert!((hsv.v - 1.0).abs() < 0.01, "value of white should be 1.0");
}

#[test]
fn test_hsv_to_rgb_roundtrip_red() {
    let (r, g, b) = hsv_to_rgb(ColorHSV {
        h: 0.0,
        s: 1.0,
        v: 1.0,
    });
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_roundtrip_green() {
    let (r, g, b) = hsv_to_rgb(ColorHSV {
        h: 120.0,
        s: 1.0,
        v: 1.0,
    });
    assert_eq!(r, 0);
    assert_eq!(g, 255);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_roundtrip_blue() {
    let (r, g, b) = hsv_to_rgb(ColorHSV {
        h: 240.0,
        s: 1.0,
        v: 1.0,
    });
    assert_eq!(r, 0);
    assert_eq!(g, 0);
    assert_eq!(b, 255);
}

#[test]
fn test_rgb_hsv_full_roundtrip() {
    let (r_in, g_in, b_in) = (200u8, 100u8, 50u8);
    let hsv = rgb_to_hsv(r_in, g_in, b_in);
    let (r_out, g_out, b_out) = hsv_to_rgb(hsv);
    assert!(
        (r_in as i32 - r_out as i32).abs() <= 2,
        "r: {} vs {}",
        r_in,
        r_out
    );
    assert!(
        (g_in as i32 - g_out as i32).abs() <= 2,
        "g: {} vs {}",
        g_in,
        g_out
    );
    assert!(
        (b_in as i32 - b_out as i32).abs() <= 2,
        "b: {} vs {}",
        b_in,
        b_out
    );
}

#[test]
fn test_rgb_to_hsl_red() {
    let hsl = rgb_to_hsl(255, 0, 0);
    assert!(
        (hsl.h - 0.0).abs() < 1.0,
        "hue should be ~0 for red, got {}",
        hsl.h
    );
    assert!((hsl.s - 1.0).abs() < 0.01, "saturation should be 1.0");
    assert!(
        (hsl.l - 0.5).abs() < 0.01,
        "lightness of pure red should be 0.5"
    );
}

#[test]
fn test_rgb_to_hsl_white() {
    let hsl = rgb_to_hsl(255, 255, 255);
    assert!(
        (hsl.l - 1.0).abs() < 0.01,
        "lightness of white should be 1.0"
    );
    assert!(
        (hsl.s - 0.0).abs() < 0.01,
        "saturation of white should be 0"
    );
}

#[test]
fn test_rgb_to_hsl_black() {
    let hsl = rgb_to_hsl(0, 0, 0);
    assert!((hsl.l - 0.0).abs() < 0.01, "lightness of black should be 0");
    assert!(
        (hsl.s - 0.0).abs() < 0.01,
        "saturation of black should be 0"
    );
}

#[test]
fn test_hsl_to_rgb_roundtrip_red() {
    let (r, g, b) = hsl_to_rgb(ColorHSL {
        h: 0.0,
        s: 1.0,
        l: 0.5,
    });
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_rgb_hsl_full_roundtrip() {
    let (r_in, g_in, b_in) = (128u8, 64u8, 192u8);
    let hsl = rgb_to_hsl(r_in, g_in, b_in);
    let (r_out, g_out, b_out) = hsl_to_rgb(hsl);
    assert!(
        (r_in as i32 - r_out as i32).abs() <= 2,
        "r: {} vs {}",
        r_in,
        r_out
    );
    assert!(
        (g_in as i32 - g_out as i32).abs() <= 2,
        "g: {} vs {}",
        g_in,
        g_out
    );
    assert!(
        (b_in as i32 - b_out as i32).abs() <= 2,
        "b: {} vs {}",
        b_in,
        b_out
    );
}
