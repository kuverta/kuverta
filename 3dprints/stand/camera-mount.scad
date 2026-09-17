// Camera mount for the triangle stand.
// A saddle, an upside-down U, that drops onto the cross bar from above. Its arms reach
// below the bar, and the camera case from thingiverse:2615180 (pi_camera_case_bottom.stl)
// hangs between them on hinge-pin.scad, looking down, with the lens about 39 mm
// along the bar from the pin.
// The pin closes the U under the bar; pressing its cap on squeezes the arms onto the
// case's knuckles, which holds the camera at the angle it is set to.
//
// Hardware: hinge-pin.scad (pin, cap and spacer), with the case's knuckles drilled
// out to 3 mm. Optional: 1x M5x10 + M5 T-nut in the bar's top slot to stop it sliding.
// Print standing on one end of the saddle; PETG, 4+ walls, 40% infill, no supports.

include <common-2020.scad>

// The case's hinge: two knuckles 2 mm wide, 4.66 mm apart, 6.4 mm across.
knuckle_d   = 6.4;
knuckle_gap = 0.2;    // play beside each of the case's knuckles; the cap takes it up
case_outer  = 4.33;   // the case's knuckles end this far either side of centre
pin_d       = 3;      // snug on the printed pin; printed holes come out a little small

drop        = 10;     // pin below the bar; the case's back then clears the bar by 3 mm
length      = 24;     // along the bar
arm_end     = 6;      // radius of the arms' rounded ends around the pin

// Bar runs along Y with its bottom face at z=0; the camera hangs toward +Y.
module pin_axis() translate([0, 0, -drop]) rotate([0, 90, 0]) children();

module saddle() {
    top = profile + clearance + wall;
    for (s = [-1, 1]) hull() {
        translate([s > 0 ? inner / 2 : -outer / 2, -length / 2, 0]) cube([wall, length, top]);
        translate([s * (inner + wall) / 2, 0, 0]) pin_axis() cylinder(r = arm_end, h = wall, center = true);
    }
    translate([-outer / 2, -length / 2, top - wall]) cube([outer, length, wall]);
}

// Bosses from each arm in to the case's knuckles, so the case hangs centred.
module bosses()
    for (s = [-1, 1]) {
        from = case_outer + knuckle_gap;
        translate([s * (from + inner / 2) / 2, 0, 0])
            pin_axis() cylinder(d = knuckle_d, h = inner / 2 - from + 0.01, center = true);
    }

difference() {
    union() { saddle(); bosses(); }
    pin_axis() cylinder(d = pin_d, h = outer + 2, center = true);
    translate([0, 0, profile]) cylinder(d = screw_d, h = wall + clearance + 2);
}
