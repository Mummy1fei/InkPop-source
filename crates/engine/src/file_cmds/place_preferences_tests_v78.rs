use super::*;

fn png_bytes() -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(200, 80, photocraft_codecs::ChannelLayout::Rgba, [230, 40, 20, 255].repeat(200 * 80)).unwrap();
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

#[test]
fn placing_follows_resize_preference_and_explicit_parameters_take_priority_v78() {
    let bytes = png_bytes();
    for depth in [8, 16, 32] {
        for linked in [false, true] {
            for (resize, params, expected_scale) in [
                (true, json!({}), 50.0),
                (false, json!({}), 100.0),
                (false, json!({"fit":true}), 50.0),
                (true, json!({"fit":false}), 100.0),
                (true, json!({"fit":true,"scale":25}), 25.0),
                (false, json!({"fit":false,"scale":25}), 25.0),
            ] {
                let mut session = Session::new();
                session.execute("file.new", json!({"width":100,"height":100,"depth":depth})).unwrap();
                session.execute("prefs.set", json!({"path":"general.resizeImageDuringPlace","value":resize})).unwrap();
                let original = session.active().unwrap().doc.clone();
                let result = place_bytes(&mut session, "Synthetic.png", bytes.clone(), linked.then(|| "Synthetic.png".to_string()), &params).unwrap();
                assert_eq!(result["scale"], json!(expected_scale), "depth={depth}, linked={linked}, resize={resize}, {params}");
                let active = session.active().unwrap();
                let layer = active.doc.layer(active.active_layer.unwrap()).unwrap();
                let LayerContent::Smart(smart) = &layer.content else { panic!("placed layer must be smart") };
                assert_eq!(smart.transform.m[0], expected_scale / 100.0);
                assert!(matches!(&smart.source, SmartSource::Linked { .. }) == linked);
                assert!(session.undo());
                assert_eq!(session.active().unwrap().doc.layers, original.layers, "one undo removes exactly the placed layer");
            }
        }
    }
}

#[test]
fn place_auto_fit_preserves_small_images_and_fits_tall_images_without_distortion_v78() {
    for (width, height, expected) in [(20, 40, 1.0), (40, 200, 0.5)] {
        let image = photocraft_codecs::Image::from_u8(
            width,
            height,
            photocraft_codecs::ChannelLayout::Rgba,
            [230, 40, 20, 255].repeat(width as usize * height as usize),
        )
        .unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let mut session = Session::new();
        session.execute("file.new", json!({"width":100,"height":100})).unwrap();
        let result = place_bytes(&mut session, "Fit.png", bytes, None, &json!({})).unwrap();
        assert_eq!(result["scale"], json!(expected * 100.0));
        let st = session.active().unwrap();
        let LayerContent::Smart(smart) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected smart layer") };
        assert_eq!(smart.transform.m[0], expected);
        assert_eq!(smart.transform.m[3], expected);
    }
}
