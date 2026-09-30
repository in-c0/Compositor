//! The JPEG metadata the import uses: the EXIF block (APP1) and an ICC profile (APP2, possibly in
//! several chunks).

#[derive(Default)]
pub(crate) struct Markers {
    /// TIFF-structured EXIF, from its byte-order mark.
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
}

pub(crate) fn read(data: &[u8]) -> Markers {
    let mut markers = Markers::default();
    let mut chunks: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut pos = 2;
    while pos + 4 <= data.len() && data[pos] == 0xff {
        let marker = data[pos + 1];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0xff {
            pos += 1;
            continue;
        }
        let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let Some(body) = data.get(pos + 4..pos + 2 + length) else { break };
        if marker == 0xe1 && body.starts_with(b"Exif\0\0") && markers.exif.is_none() {
            markers.exif = Some(body[6..].to_vec());
        }
        if marker == 0xe2 && body.starts_with(b"ICC_PROFILE\0") && body.len() > 14 {
            chunks.push((body[12], body[14..].to_vec()));
        }
        pos += 2 + length;
    }
    if !chunks.is_empty() {
        chunks.sort_by_key(|c| c.0);
        markers.icc = Some(chunks.into_iter().flat_map(|c| c.1).collect());
    }
    markers
}
