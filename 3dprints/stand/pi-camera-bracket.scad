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
// 4x M3x6 self-tapping into the posts for the Pi.
// Print standing on its back, the face with the two M5 holes; PETG, 4+ walls,
// 40% infill, no supports.

include <common-2020.scad>

// Bar runs along Y with its bottom face at z=0. The mount grips its +X side.
back      = inner / 2 + wall;      // outer face of the part against the bar's side
top_z     = profile + clearance;   // underside of the plate over the bar
closed    = false;                 // a fourth wall on the open side: a ring the bar has
                                   // to be pushed through; see pi-camera-bracket-closed.scad

// Pi Zero: 65 x 30 board, holes 58 x 23 apart; M3 screws go through them.
// pi4-camera-bracket.scad sets these for a Pi 4 and reuses everything else.
pi_board  = [30, 65];
pi_tilt   = 0;                // degrees the Pi leans up toward the open side; see
                              // pi4-camera-bracket-tilted.scad
pi_holes  = [23, 58];
post_h    = 3;
post_d    = 6;
post_hole = 2.9;              // for M3 self-tapping; printed holes come out ~0.3 mm small
// Centre of the holes, across the bar: on the bar's centre line, unless that puts
// a post out past the back face the part stands on to print.
pi_x      = min(0, back - post_d / 2 - pi_holes[0] / 2);
plate     = [[-17, back], [-34, 34]];

// Camera board: 25 x 25 as measured, lens looking down through the window.
// The window runs out to the way in as a channel, so the lens barrel and whatever
// else stands out of the board's underside slide along it instead of hitting the floor.
// The board lies on the floor of the slot under its own weight, so slack above it
// costs nothing but makes it go in.
board     = [25, 25];
cam_board = board + [0.6, 0.6]; // the slot: 0.3 mm of play on each side, so the
                            // board goes in without room to rattle; in along +Y
// A closed ring cannot flex to let the board's cable connector squeeze under the
// lips, so its slot is 2 mm taller, and the camera block sits 2 mm lower to keep the
// same material between the slot and the bar.
board_t   = closed ? 5.5 : 3.5; // board, plus what is soldered within reach of the lips
window    = [21.6, 18];     // what the camera looks through, and the channel's width;
                            // what is left either side is the 2 mm ledge the board rests on
tray_z    = closed ? -14 : -12; // underside of the tray
floor_t   = 2;
rim       = 1.2;            // material around the slot, beside it and behind it
lip       = 2;              // how far the lips reach in over the board's edges

// The tray keeps `rim` of wall on the back side too, sitting over if the slot is
// wider than the room between the bar's side and the back of this part. The block
// is then the slot plus a rim each side, which is what makes it 28 mm across.
cam_x     = min(0, -(cam_board[0] / 2 + rim - back));

web_y     = 20;
bar_screws = [-13, 13];

slot_z    = tray_z + floor_t;

module plate() translate([plate[0][0], plate[1][0], top_z])
    cube([plate[0][1] - plate[0][0], plate[1][1] - plate[1][0], wall]);

// What sits on the bar: the plate's width over it, which stays flat however the
// Pi leans.
module roof() translate([-outer / 2, plate[1][0], top_z])
    cube([back + outer / 2, plate[1][1] - plate[1][0], wall]);

// The Pi's plate, leaned up by pi_tilt about its top edge along the back face.
// At no tilt this changes nothing.
module pi_frame() translate([back, 0, top_z + wall]) rotate([0, pi_tilt, 0])
    translate([-back, 0, -(top_z + wall)]) children();

// The leaned plate, and a wedge filling in under it over the bar.
module pi_plate() {
    pi_frame() plate();
    hull() { roof(); pi_frame() roof(); }
}

// Sunk half a millimetre into the plate rather than standing on it: once leaned,
// faces that only touch no longer quite meet, and the union leaves a seam.
module posts() pi_frame() for (x = [-1, 1], y = [-1, 1])
    translate([pi_x + x * pi_holes[0] / 2, y * pi_holes[1] / 2, top_z + wall - 0.5])
        cylinder(d = post_d, h = post_h + 0.5, $fn = 32);

module web() translate([inner / 2, -web_y, tray_z]) cube([wall, 2 * web_y, top_z + wall - tray_z]);

// The same wall on the open side, closing the C into a ring round the bar, and the
// gap under the bar filled down to the camera block, so the bar has the same
// clearance underneath as on its other three sides.
module side_wall() {
    translate([-inner / 2 - wall, -web_y, tray_z])
        cube([wall, 2 * web_y, top_z + wall - tray_z]);
    translate([-inner / 2 - wall, -web_y, tray_z + floor_t + board_t])
        cube([inner + 2 * wall, 2 * web_y, -(tray_z + floor_t + board_t)]);
}

module tray() translate([cam_x - cam_board[0] / 2 - rim, -cam_board[1] / 2 - rim, tray_z])
    cube([cam_board[0] / 2 + rim + back - cam_x, cam_board[1] + 2 * rim,
          floor_t + board_t + lip]);

difference() {
    union() { pi_plate(); posts(); web(); tray(); if (closed) side_wall(); }

    // the bar itself, and the slot the camera board slides into
    // Open on the far side, unless the ring is closed: then only the bar's own size,
    // with clearance underneath as well.
    open_to = closed ? inner / 2 : far;
    below   = closed ? clearance : 0;
    translate([-open_to, -far, -below]) cube([open_to + inner / 2, 2 * far, top_z + below]);
    // The slot: a plain rectangle, straight in from the mouth to the back wall.
    translate([cam_x - cam_board[0] / 2, -cam_board[1] / 2, slot_z])
        cube([cam_board[0], far, board_t]);
    // over the board: open, apart from a lip along each side
    translate([cam_x - cam_board[0] / 2 + lip, -cam_board[1] / 2 + lip, slot_z + board_t])
        cube([cam_board[0] - 2 * lip, far, lip + 1]);
    // what the camera looks through
    translate([cam_x - window[0] / 2, -window[1] / 2, tray_z - 1])
        cube([window[0], far, floor_t + 2]);

    for (y = bar_screws) translate([inner / 2 - 1, y, profile / 2]) rotate([0, 90, 0]) cylinder(d = screw_d, h = wall + 2);
    pi_frame() for (x = [-1, 1], y = [-1, 1])
        translate([pi_x + x * pi_holes[0] / 2, y * pi_holes[1] / 2, top_z - 1])
            cylinder(d = post_hole, h = wall + post_h + 2, $fn = 24);
}
