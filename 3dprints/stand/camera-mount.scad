// Camera mount for the triangle stand: the camera alone, hanging under the cross bar.
// For the Pi and the camera together on one part, use pi-camera-mount.scad instead.
//
// Hardware: hinge-pin.scad (pin, cap and spacer), with the case's knuckles drilled
// out to 3 mm. Optional: 1x M5x10 + M5 T-nut in the bar's top slot to stop it sliding.
// Print standing on one end of the saddle; PETG, 4+ walls, 40% infill, no supports.

include <saddle-2020.scad>

length = 24;   // along the bar

difference() {
    union() { saddle(-length / 2, length / 2, 0); bosses(0); }
    pin_hole(0);
    lock_screw(0);
}
