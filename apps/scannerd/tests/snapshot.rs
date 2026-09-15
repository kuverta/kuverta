//! The picture setup chooses the crop on, and photographs of any crop shape.

use std::sync::Arc;

use scannerd::camera::RpiCamera;
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
    hub.set_full_view(jpeg.clone(), Some("0.200,0.100,0.500,0.700".into()));

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
