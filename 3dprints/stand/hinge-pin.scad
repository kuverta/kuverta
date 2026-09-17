// Printed hinge pin for camera-mount.scad.
// Drill the camera case's two knuckles out to 3 mm. Push the pin through one arm,
// the case's first knuckle, the spacer, the case's second knuckle and the other arm,
// then press the cap onto its end: that squeezes the arms onto the case, so the
// hinge holds the camera where it is set.
// Print lying down on the flats, PETG, 100% infill, no supports.

include <common-2020.scad>

pin_d   = 3;
pin_len = outer + 2;   // through both of the mount's arms, plus the cap
head_d  = 6;
head_h  = 1.6;
cap_h   = 2;
press   = 0.2;      // how much tighter the cap's hole is than the pin
flat    = 0.4;      // how much comes off the bottom, so it lies still on the bed
tip     = 0.4;      // chamfer that leads the pin into the holes
spacer  = 4.26;     // fills the 4.66 mm gap between the case's knuckles, less play

module pin() {
    cylinder(d = head_d, h = head_h);
    translate([0, 0, head_h]) {
        cylinder(d = pin_d, h = pin_len - tip);
        translate([0, 0, pin_len - tip]) cylinder(d1 = pin_d, d2 = pin_d - 2 * tip, h = tip);
    }
}

module ring(h, hole) difference() {
    cylinder(d = head_d, h = h);
    translate([0, 0, -1]) cylinder(d = hole, h = h + 2);
}

// Pin lies along X, cut flat underneath; the cap and spacer stand on their faces beside it.
difference() {
    translate([0, 0, pin_d / 2 - flat]) rotate([0, 90, 0]) pin();
    translate([-50, -50, -100]) cube(100);
}
translate([0, head_d + 3, 0]) ring(cap_h, pin_d - press);
translate([head_d + 3, head_d + 3, 0]) ring(spacer, pin_d + 0.3);
