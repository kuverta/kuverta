// Shared geometry for the mounts that hang on the cross bar.
// A saddle is an upside-down U that drops onto the bar from above, with arms reaching
// below it. A pin through the arms carries the camera case from thingiverse:2615180
// (pi_camera_case_bottom.stl) on its own hinge, so the camera looks down.
// The bar runs along Y with its bottom face at z=0; the camera hangs toward +Y.

include <common-2020.scad>

// The case's hinge: two knuckles 2 mm wide, 4.66 mm apart, 6.4 mm across.
knuckle_d   = 6.4;
knuckle_gap = 0.2;    // play beside each of the case's knuckles; the cap takes it up
case_outer  = 4.33;   // the case's knuckles end this far either side of centre
pin_d       = 3;      // snug on the printed pin; printed holes come out a little small

drop        = 10;     // pin below the bar; the case's back then clears the bar by 3 mm
arm_end     = 6;      // radius of the arms' rounded ends around the pin

saddle_top  = profile + clearance + wall;

module pin_axis(y) translate([0, y, -drop]) rotate([0, 90, 0]) children();

// The U itself, from `from` to `to` along the bar, with its arms drawn out to the pin.
module saddle(from, to, pin_y) {
    for (s = [-1, 1]) hull() {
        translate([s > 0 ? inner / 2 : -outer / 2, from, 0]) cube([wall, to - from, saddle_top]);
        translate([s * (inner + wall) / 2, 0, 0]) pin_axis(pin_y) cylinder(r = arm_end, h = wall, center = true);
    }
    translate([-outer / 2, from, saddle_top - wall]) cube([outer, to - from, wall]);
}

// Bosses from each arm in to the case's knuckles, so the case hangs centred.
module bosses(pin_y)
    for (s = [-1, 1]) {
        from = case_outer + knuckle_gap;
        translate([s * (from + inner / 2) / 2, 0, 0])
            pin_axis(pin_y) cylinder(d = knuckle_d, h = inner / 2 - from + 0.01, center = true);
    }

module pin_hole(pin_y) pin_axis(pin_y) cylinder(d = pin_d, h = outer + 2, center = true);

// M5 into a T-nut in the bar's top slot, so the saddle cannot slide along the bar.
module lock_screw(y) translate([0, y, profile]) cylinder(d = screw_d, h = wall + clearance + 2);
