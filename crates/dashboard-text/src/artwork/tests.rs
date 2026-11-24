use super::*;

fn image(width: u32, height: u32, value: u8) -> Rgba8Image {
    Rgba8Image {
        width,
        height,
        pixels: vec![value; (width * height * 4) as usize],
    }
}

fn decoded(png: &[u8]) -> Rgba8Image {
    let mut reader = png::Decoder::new(io::Cursor::new(png)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(frame.buffer_size());
    Rgba8Image {
        width: frame.width,
        height: frame.height,
        pixels,
    }
}

#[test]
fn urls_round_trip_their_image() {
    let drawn = image(3, 2, 200);
    let url = png_url(&drawn).unwrap();
    let name = url.strip_prefix(base()).unwrap();
    assert_eq!(name.len(), NAME_LENGTH);
    let png = url_bytes(&url).unwrap();
    assert_eq!(bytes(name).unwrap(), png);
    assert_eq!(decoded(&png).pixels, drawn.pixels);
}

#[test]
fn identical_pixels_are_one_name() {
    let table = Table::new("test:");
    let first = table.put(encode(&image(4, 4, 7)).unwrap());
    assert_eq!(first, table.put(encode(&image(4, 4, 7)).unwrap()));
    assert_ne!(first, table.put(encode(&image(4, 4, 8)).unwrap()));
    assert_eq!(table.read().len(), 2);
}

#[test]
fn names_are_validated_strictly() {
    let table = Table::new("/api/artwork/");
    let url = table.put(encode(&image(2, 2, 9)).unwrap());
    let name = url.strip_prefix("/api/artwork/").unwrap().to_owned();
    assert!(table.bytes(&name).is_some());
    for refused in [
        format!("../{name}"),
        format!("..\\{name}"),
        format!("/{name}"),
        format!("{name}/"),
        format!("{name}\0"),
        format!("a{name}"),
        name[1..].to_owned(),
        name.replace(".png", ".PNG"),
        name.to_uppercase(),
        name.replace(".png", ".tmp"),
        String::new(),
        "..".into(),
    ] {
        assert!(table.bytes(&refused).is_none(), "{refused:?}");
    }
    assert!(table.url_bytes(&format!("http://artwork.localhost/{name}")).is_none());
    assert!(table.url_bytes(&url).is_some());
    assert!(table.bytes(&format!("{}.png", "0".repeat(32))).is_none());
}

#[test]
fn every_image_handed_out_stays_served() {
    let table = Table::new("test:");
    let urls: Vec<_> = (0..64)
        .map(|value| table.put(encode(&image(16, 16, value)).unwrap()))
        .collect();
    for (value, url) in urls.iter().enumerate() {
        assert_eq!(decoded(&table.url_bytes(url).unwrap()).pixels[0], value as u8);
    }
}

#[test]
fn text_carries_each_image_it_names_once() {
    let table = Table::new("test:");
    let first = encode(&image(2, 1, 3)).unwrap();
    let second = encode(&image(2, 1, 4)).unwrap();
    let (one, two) = (table.put(first.clone()), table.put(second.clone()));
    assert_eq!(table.images("no images").unwrap(), Vec::<Arc<[u8]>>::new());
    assert!(table.images("test:not-an-image test:").unwrap().is_empty());
    let mut carried = table
        .images(&format!(r#"["{one}","{two}","{one}"]"#))
        .unwrap()
        .iter()
        .map(|png| png.to_vec())
        .collect::<Vec<_>>();
    carried.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(carried, expected);
    let missing = format!("test:{}.png", "ab".repeat(16));
    assert!(table.images(&format!("{one} {missing}")).is_none());
}

#[test]
fn restoring_keeps_only_the_images_the_text_names() {
    let source = Table::new("test:");
    let shown = encode(&image(3, 3, 1)).unwrap();
    let other = encode(&image(3, 3, 2)).unwrap();
    let text = format!(r#"{{"icon":"{}"}}"#, source.put(shown.clone()));
    let fresh = Table::new("test:");
    assert!(!fresh.restore(Vec::<Vec<u8>>::new(), &text));
    assert!(fresh.restore([other, shown.clone()], &text));
    assert_eq!(*fresh.url_bytes(&text[9..text.len() - 2]).unwrap(), *shown);
    assert_eq!(fresh.read().len(), 1, "an image the text does not name is not kept");
    let mut damaged = shown;
    *damaged.last_mut().unwrap() ^= 1;
    assert!(!Table::new("test:").restore([damaged], &text));
    assert!(fresh.restore(Vec::<Vec<u8>>::new(), "no images"));
}

#[test]
fn data_urls_carry_their_image_inline() {
    let url = png_data_url(&image(3, 2, 255)).unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(url.strip_prefix("data:image/png;base64,").unwrap())
        .unwrap();
    assert_eq!((decoded(&png).width, decoded(&png).height), (3, 2));
}

#[test]
fn a_later_table_serves_the_names_an_earlier_one_kept() {
    let folder = std::env::temp_dir().join(format!("artwork-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let png = encode(&image(2, 2, 9)).unwrap();
    let url = Table::kept_in("test:", &folder).put(png.clone());
    let name = url.strip_prefix("test:").unwrap();
    let later = Table::kept_in("test:", &folder);
    assert_eq!(&*later.bytes(name).unwrap(), &png[..]);
    assert!(later.restore(Vec::new(), &url), "a reply naming it is whole");
    let other = Name::of(&encode(&image(2, 2, 10)).unwrap()).to_string();
    std::fs::write(folder.join(&other), &png).unwrap();
    assert!(later.bytes(&other).is_none());
    assert!(Table::new("test:").bytes(name).is_none(), "a table without a folder keeps memory alone");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn an_embedded_viewers_images_are_named_and_served_as_the_desktops() {
    let folder = std::env::temp_dir().join(format!("artwork-desktop-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let png = encode(&image(2, 2, 5)).unwrap();
    let url = Table::kept_in(DESKTOP_BASE, &folder).put(png.clone());
    let name = url.strip_prefix("http://artwork.localhost/").unwrap();
    assert!(Name::parse(name).is_some(), "{url}");
    let desktop = Table::kept_in(DESKTOP_BASE, &folder);
    assert_eq!(&*desktop.url_bytes(&url).unwrap(), &png[..], "the desktop serves what the viewer drew");
    assert!(desktop.url_bytes(&Table::new(UNCONFIGURED).put(png)).is_none(), "an unconfigured process's names are never served");
    std::fs::remove_dir_all(&folder).unwrap();
}
