//! The picture setup chooses the crop on, and photographs of any crop shape.

use std::sync::Arc;

use scannerd::camera::{RpiCamera, Sensor};
use scannerd::hub::Hub;
use scannerd::web::{serve, Web};

#[test]
fn a_crop_is_photographed_at_its_own_share_of_the_sensor() {
    // A page-shaped crop: taller than it is wide. Scaled to the camera's full
    // 4:3 output instead, the page came out stretched.
    let camera = RpiCamera {
        roi: Some("0.25,0.1,0.5,0.75".into()),
        ..RpiCamera::default()
    };
    assert_eq!(camera.capture_size(), Some((1296, 1458)));

    // Odd sizes are rounded to even ones, which every encoder takes.
    let odd = RpiCamera {
        roi: Some("0,0,0.3333,0.3333".into()),
        ..RpiCamera::default()
    };
    let (width, height) = odd.capture_size().unwrap();
    assert_eq!((width % 2, height % 2), (0, 0));

    assert_eq!(
        RpiCamera::default().capture_size(),
        None,
        "no crop, the camera's own size"
    );
    let nonsense = RpiCamera {
        roi: Some("left half".into()),
        ..RpiCamera::default()
    };
    assert_eq!(nonsense.capture_size(), None);
}

#[test]
fn another_sensor_gives_another_size() {
    let v3 = RpiCamera {
        roi: Some("0,0,0.5,0.5".into()),
        sensor_width: 4608,
        sensor_height: 2592,
        ..RpiCamera::default()
    };
    assert_eq!(v3.capture_size(), Some((2304, 1296)));
}

#[test]
fn the_camera_says_which_sensor_it_is() {
    // What `rpicam-still --list-cameras` prints on the rig, for its Pi camera
    // v2.
    let v2 = "\
Available cameras
-----------------
0 : imx219 [3280x2464 10-bit] (/base/soc/i2c0mux/i2c@1/imx219@10)
    Modes: 'SBGGR10_CSI2P' : 640x480 [30.00 fps - (65527, 65527)/65535x65535 crop]
                             1640x1232 [30.00 fps - (65527, 65527)/65535x65535 crop]
                             1920x1080 [30.00 fps - (65527, 65527)/65535x65535 crop]
                             3280x2464 [30.00 fps - (65527, 65527)/65535x65535 crop]
           'SBGGR8' : 640x480 [30.00 fps - (65527, 65527)/65535x65535 crop]
";
    assert_eq!(
        Sensor::parse(v2),
        Some(Sensor {
            model: "imx219".into(),
            width: 3280,
            height: 2464,
        })
    );

    // The v1, and the older listing with nothing but the size in brackets.
    let v1 = Sensor::parse("0 : ov5647 [2592x1944] (/base/soc/i2c0mux/i2c@1/ov5647@36)").unwrap();
    assert_eq!(
        (v1.model.as_str(), v1.width, v1.height),
        ("ov5647", 2592, 1944)
    );

    // With two, the first: the one photographed with when none is chosen.
    let two = "\
0 : imx708 [4608x2592 10-bit RGGB] (/base/axi/pcie@120000/rp1/i2c@88000/imx708@1a)
1 : ov5647 [2592x1944 10-bit GBRG] (/base/axi/pcie@120000/rp1/i2c@80000/ov5647@36)
";
    assert_eq!(Sensor::parse(two).unwrap().model, "imx708");

    assert_eq!(Sensor::parse("No cameras available!"), None);
    assert_eq!(Sensor::parse(""), None);
}

#[test]
fn a_camera_is_asked_through_its_program() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("scannerd-sensor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program_in = |name: &str, script: &str| {
        let program = dir.join(name);
        std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program.to_string_lossy().into_owned()
    };

    let v2 = program_in(
        "lists-a-v2",
        r#"[ "$1" = --list-cameras ] || exit 2
echo "Available cameras"
echo "0 : imx219 [3280x2464 10-bit RGGB] (/base/soc/i2c0mux/i2c@1/imx219@10)""#,
    );
    let sensor = Sensor::detect(&v2).unwrap();
    assert_eq!((sensor.width, sensor.height), (3280, 2464));

    // A crop on it is photographed at the v2's pixels, not the v1's.
    let camera = RpiCamera {
        roi: Some("0,0,0.5,0.5".into()),
        sensor_width: sensor.width,
        sensor_height: sensor.height,
        ..RpiCamera::default()
    };
    assert_eq!(camera.capture_size(), Some((1640, 1232)));

    let none = program_in("lists-nothing", "echo 'No cameras available!'");
    let error = Sensor::detect(&none).unwrap_err().to_string();
    assert!(error.contains("lists no camera"), "{error}");

    // The ffmpeg stand-in, and anything else that does not know the flag.
    let deaf = program_in("knows-no-such-flag", "echo 'unsupported flag' >&2; exit 2");
    let error = Sensor::detect(&deaf).unwrap_err().to_string();
    assert!(error.contains("unsupported flag"), "{error}");

    assert!(Sensor::detect(&dir.join("not-there").to_string_lossy()).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn the_picture_of_the_whole_view_is_served_with_where_the_page_was_found() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let hub = Arc::new(Hub::new(320, 240));
    let dir = std::env::temp_dir();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(serve(listener, Arc::new(Web::new(hub.clone(), dir, None))));
    let client = reqwest::Client::new();
    let url = format!("{base}/full-view.jpg");

    assert_eq!(client.get(&url).send().await.unwrap().status(), 404);

    let jpeg = b"\xff\xd8a picture of a table".to_vec();
    hub.set_full_view(jpeg.clone(), Some("0.200,0.100,0.500,0.700".into()), None);

    let reply = client.get(&url).send().await.unwrap();
    assert_eq!(reply.headers()["content-type"], "image/jpeg");
    assert_eq!(reply.bytes().await.unwrap().to_vec(), jpeg);

    let status = hub.status();
    assert_eq!(status.full_view_at, 1);
    assert_eq!(
        status.suggested_crop.as_deref(),
        Some("0.200,0.100,0.500,0.700")
    );
}
