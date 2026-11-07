#![cfg(windows)]

use game_helper_windows::window::ClientArea;

fn area() -> ClientArea {
    ClientArea {
        hwnd: 0,
        screen_x: -1920,
        screen_y: 120,
        width: 1920,
        height: 1080,
    }
}

#[test]
fn maps_negative_monitor_origin_and_last_client_pixel() {
    let area = area();
    assert_eq!(area.normalize_screen(-1920, 120), Some((0.0, 0.0)));
    assert_eq!(area.normalize_screen(-1, 1199), Some((1.0, 1.0)));
}

#[test]
fn rejects_points_outside_instead_of_clamping_to_a_valid_route() {
    let area = area();
    for (x, y) in [(-1921, 120), (0, 120), (-1920, 119), (-1, 1200)] {
        assert_eq!(area.normalize_screen(x, y), None);
    }
}

#[test]
fn rejects_degenerate_client_sizes() {
    for (width, height) in [(0, 1080), (1920, 0), (1, 1080), (1920, 1)] {
        let area = ClientArea {
            width,
            height,
            ..area()
        };
        assert_eq!(area.normalize_screen(-1920, 120), None);
    }
}
