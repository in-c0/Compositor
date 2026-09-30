use crate::develop::unpremultiply;

#[test]
fn exif_orientation_reads_both_byte_orders() {
    let big = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    assert_eq!(crate::exif::orientation(big), Some(6));
    let little = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x08\0\0\0\0\0\0\0";
    assert_eq!(crate::exif::orientation(little), Some(8));
}

#[test]
fn premultiplying_a_stored_pixel_gives_back_the_premultiplied_one() {
    // The project stores what ImageIO writes; the renderer premultiplies it again with rounding.
    for a in 1..=255u32 {
        for c in 0..=a {
            let straight = unpremultiply([c as u8, 0, 0, a as u8]);
            assert_eq!((straight[0] as u32 * a + 127) / 255, c, "c {c} a {a}");
        }
    }
}

#[test]
fn temperature_lands_on_the_planckian_locus() {
    // A 6500 K black body is at (0.3135, 0.3236).
    let xy = crate::raw::temperature_to_xy(6500.0, 0.0);
    assert!((xy[0] - 0.3135).abs() < 0.0005 && (xy[1] - 0.3236).abs() < 0.0005, "{xy:?}");
}

#[test]
fn gifs_are_refused_as_the_app_refuses_them() {
    let gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
    assert!(matches!(crate::import(gif, "gif", "x", None), Err(crate::ImportError::Unsupported)));
}
