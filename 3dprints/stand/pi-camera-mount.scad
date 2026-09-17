// Mount for the Pi and the camera together, on the cross bar of the triangle stand.
// The saddle drops onto the bar from above. Its top is a shelf that the Pi case from
// thingiverse:2615180 (pi_bottom_cover.stl) bolts onto through the two M4 holes it
// already has for 2020 profile, 50 mm apart. The camera case hangs from the pin below
// the bar, at the +Y end, so the lens looks down about 39 mm further along the bar.
//
// Hardware: 2x M4x12 + M4 nuts for the Pi case (the nuts go under the shelf, beside
// the bar), hinge-pin.scad for the camera, with the case's knuckles drilled out to
// 3 mm. Optional: 1x M5x10 + M5 T-nut in the bar's top slot to stop it sliding; fit
// that one before the Pi case, which covers it.
// Print lying on the shelf, arms up; PETG, 4+ walls, 40% infill, no supports.

include <saddle-2020.scad>

from     = -40;
to       = 32;
pin_y    = 25;          // camera hinge, at the far end from the Pi
lock_y   = -5;

pi_screw = 4.3;         // M4 clearance
pi_holes = [-30, 20];   // 50 mm apart, as on the case's plate
pi_x     = 19;          // clear of the arms, so an M4 nut fits under the shelf
shelf    = [-26, 25];   // across the bar; holds up the Pi case, which overhangs a little

module shelf() translate([shelf[0], from, saddle_top - wall])
    cube([shelf[1] - shelf[0], to - from, wall]);

difference() {
    union() { saddle(from, to, pin_y); shelf(); bosses(pin_y); }
    pin_hole(pin_y);
    lock_screw(lock_y);
    for (y = pi_holes) translate([pi_x, y, saddle_top - wall - 1]) cylinder(d = pi_screw, h = wall + 2);
}
