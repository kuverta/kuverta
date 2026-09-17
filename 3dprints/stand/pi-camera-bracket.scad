// One-piece mount for the Pi and the camera on the cross bar of the triangle stand.
// A C that wraps the bar: the Pi Zero screws onto the plate on top, the camera board
// slides into the slot underneath, looking down through the window, on the bar's
// centre line. The open side leaves room for the ribbon cable to run from the camera
// up to the Pi, and lets the mount come off the bar without taking the frame apart.
//
// The boards are held directly, so the cases from thingiverse:2615180 are not needed.
// Board sizes are the published ones for a Pi Zero and a Pi camera module; check them
// against yours, and change pi_holes / cam_board / board_t if they differ.
//
// Hardware: 2x M5x10 button head + 2x M5 T-nuts (slot 6) into the bar's side slot,
// 4x M2.5x6 self-tapping into the posts for the Pi.
// Print standing on its back, the face with the two M5 holes; PETG, 4+ walls,
// 40% infill, no supports.

include <common-2020.scad>

// Bar runs along Y with its bottom face at z=0. The mount grips its +X side.
back      = inner / 2 + wall;      // outer face of the part against the bar's side
top_z     = profile + clearance;   // underside of the plate over the bar

// Pi Zero: 65 x 30 board, holes 58 x 23 apart, M2.5.
pi_holes  = [23, 58];
post_h    = 3;
post_d    = 6;
post_hole = 2.2;
plate     = [[-17, back], [-34, 34]];

// Camera board: 25 x 24, about 1 mm thick, lens looking down through the window.
cam_board = [25.6, 24.8];   // with clearance; it slides in along +Y
board_t   = 1.4;
window    = [16, 18];
tray_z    = -12;            // underside of the tray
floor_t   = 2;
lip       = 3;              // how far the lips reach in over the board's edges
bump      = 0.3;            // stops the board sliding back out

web_y     = 20;
bar_screws = [-13, 13];

slot_z    = tray_z + floor_t;

module plate() translate([plate[0][0], plate[1][0], top_z])
    cube([plate[0][1] - plate[0][0], plate[1][1] - plate[1][0], wall]);

module posts() for (x = [-1, 1], y = [-1, 1])
    translate([x * pi_holes[0] / 2, y * pi_holes[1] / 2, top_z + wall])
        cylinder(d = post_d, h = post_h, $fn = 32);

module web() translate([inner / 2, -web_y, tray_z]) cube([wall, 2 * web_y, top_z + wall - tray_z]);

module tray() translate([-cam_board[0] / 2 - lip, -cam_board[1] / 2 - lip, tray_z])
    cube([cam_board[0] / 2 + lip + back, cam_board[1] + 2 * lip, floor_t + board_t + wall / 2]);

difference() {
    union() { plate(); posts(); web(); tray(); }

    // the bar itself, and the slot the camera board slides into
    translate([-far, -far, 0]) cube([far + inner / 2, 2 * far, top_z]);
    translate([-cam_board[0] / 2, -cam_board[1] / 2, slot_z]) cube([cam_board[0], far, board_t]);
    // over the board: open, apart from a lip along each side
    translate([-cam_board[0] / 2 + lip, -cam_board[1] / 2 + lip, slot_z + board_t])
        cube([cam_board[0] - 2 * lip, far, wall]);
    // what the camera looks through
    translate([-window[0] / 2, -window[1] / 2, tray_z - 1]) cube([window[0], window[1], floor_t + 2]);

    for (y = bar_screws) translate([inner / 2 - 1, y, profile / 2]) rotate([0, 90, 0]) cylinder(d = screw_d, h = wall + 2);
    for (x = [-1, 1], y = [-1, 1])
        translate([x * pi_holes[0] / 2, y * pi_holes[1] / 2, top_z - 1])
            cylinder(d = post_hole, h = wall + post_h + 2, $fn = 24);
}

// A bump under each edge of the board, near the way in, to keep it from sliding out.
for (x = [-1, 1])
    translate([x * (cam_board[0] / 2 - 2), cam_board[1] / 2 - 2, slot_z])
        cylinder(d = 2, h = bump, $fn = 16);
