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
    let sensor = Sensor::parse(v2).unwrap();
    assert_eq!(
        (sensor.model.as_str(), sensor.width, sensor.height),
        ("imx219", 3280, 2464)
    );
    assert_eq!(
        sensor.modes,
        [(640, 480), (1640, 1232), (1920, 1080), (3280, 2464)],
        "each mode once, whatever formats it comes in"
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
    let first = Sensor::parse(two).unwrap();
    assert_eq!(first.model, "imx708");
    assert_eq!(
        first.modes,
        [],
        "the second camera's modes are not the first's"
    );

    assert_eq!(Sensor::parse("No cameras available!"), None);
    assert_eq!(Sensor::parse(""), None);
}

#[test]
fn the_whole_sensor_is_looked_at_and_not_its_middle() {
    // The v2's 640×480 is the middle two fifths of the sensor: left to choose
    // for a 320×240 preview, the camera program took it, and the rig showed
    // the middle of a letter. Half the sensor each way is all of it.
    let v2 = Sensor {
        model: "imx219".into(),
        width: 3280,
        height: 2464,
        modes: vec![(640, 480), (1640, 1232), (1920, 1080), (3280, 2464)],
    };
    assert_eq!(v2.view_mode(), (1640, 1232));
    assert_eq!(v2.capture_mode(), (3280, 2464));

    // A sensor with no such mode is read whole, which is dearer and right.
    let plain = Sensor {
        modes: vec![(640, 480), (3280, 2464)],
        ..v2.clone()
    };
    assert_eq!(plain.view_mode(), (3280, 2464));
    let unlisted = Sensor {
        modes: Vec::new(),
        ..v2
    };
    assert_eq!(unlisted.view_mode(), (3280, 2464));
}

#[test]
fn every_picture_is_taken_in_its_mode() {
    use scannerd::camera::Camera;
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("scannerd-mode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let said = dir.join("said");
    // Writes down what it was asked, and answers with a frame, a JPEG's first
    // bytes or a file, as asked.
    let program = dir.join("fake-rpicam-still");
    std::fs::write(
        &program,
        format!(
            r#"#!/bin/sh
echo "$@" >> '{}'
out=; jpeg=; last=
for arg in "$@"; do
    [ "$last" = --output ] && out=$arg
    [ "$arg" = jpg ] && jpeg=yes
    last=$arg
done
if [ "$out" != - ]; then : > "$out"
elif [ -n "$jpeg" ]; then printf '\377\330'
else head -c 76800 /dev/zero
fi
"#,
            said.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let asked = |camera: &RpiCamera| {
        let _ = std::fs::remove_file(&said);
        camera.preview().unwrap();
        camera.full_view().unwrap();
        camera.snapshot().unwrap();
        camera.capture(&dir.join("page.jpg")).unwrap();
        std::fs::read_to_string(&said).unwrap()
    };

    let camera = RpiCamera {
        program: program.to_string_lossy().into_owned(),
        roi: Some("0.1,0.1,0.8,0.8".into()),
        view_mode: Some((1640, 1232)),
        capture_mode: Some((3280, 2464)),
        ..RpiCamera::default()
    };
    let said_in_modes = asked(&camera);
    let lines: Vec<&str> = said_in_modes.lines().collect();
    assert_eq!(lines.len(), 4);
    for (line, what) in lines.iter().zip(["preview", "whole view", "snapshot"]) {
        assert!(line.contains("--mode 1640:1232"), "{what}: {line}");
    }
    assert!(
        lines[3].contains("--mode 3280:2464"),
        "photograph: {}",
        lines[3]
    );

    // A camera nobody could ask is left to choose, as it always was.
    let unknown = RpiCamera {
        view_mode: None,
        capture_mode: None,
        ..camera
    };
    assert!(!asked(&unknown).contains("--mode"));
    std::fs::remove_dir_all(&dir).unwrap();
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
