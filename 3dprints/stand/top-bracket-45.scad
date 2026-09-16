// Top bracket for the scanner stand.
// Caps the vertical 2020 post and holds the camera arm so it leaves the post
// rising up and outward at arm_rise above horizontal, toward the middle of the page.
//
// Hardware: 8x M5x10 button head + 8x M5 T-nuts (slot 6), 2 per side per profile.
// Optional: 1x M5x12 into the post's tapped centre bore; fit it before the arm,
// tighten through the hex key hole in the arm socket's top wall.
// Print lying on one of the large flat sides, PETG, 4+ walls, 40% infill, no supports.

include <common-2020.scad>

arm_rise    = 45;     // angle of the arm above horizontal
arm_offset  = [0, 0, 14]; // where the arm's end sits; keeps the arm clear of the post
core_screw  = true;

// Post runs down -Z from the origin.
module post_frame() children();

// Arm socket is the same socket, tilted so its -Z points up and away from the post.
module arm_frame() translate(arm_offset) rotate([0, -(90 + arm_rise), 0]) children();

difference() {
    hull() {
        post_frame() socket_body();
        arm_frame() socket_body();
    }
    post_frame() { socket_void(); socket_screws(); }
    arm_frame()  { socket_void(); socket_screws(); }
    if (core_screw) {
        translate([0, 0, -1]) cylinder(d = screw_d, h = far);
        translate([0, 0, wall]) cylinder(d = 10, h = arm_offset[2] - wall);
        cylinder(d = 6, h = far); // hex key access through the arm socket
    }
}
