// Shared geometry for the triangle stand: two upright triangles of 2020 profile,
// joined at their apexes by a cross bar that carries the camera, the Pi and a light.
// Each triangle lies in the XZ plane; the corner of a bracket is where the profiles'
// centre lines meet, at the origin.

include <common-2020.scad>

leg_rise = 60;   // angle of a leg above the base; 60 makes the triangles equilateral

// How far a profile's end stays back from the corner, so two profiles meeting at
// `between` degrees clear each other with a little wall left between them.
function setback(between) = (inner / 2) / tan(between / 2) + 2;

// Socket for a profile leaving the corner at `a` degrees from +X toward +Z.
// Its screws run along Y, through the bracket's flat front and back faces.
module bar_frame(a, s) rotate([0, -(90 + a), 0]) translate([0, 0, -s]) children();

// Two sockets `between` degrees apart, filled in between.
module vee(a1, a2) {
    s = setback(abs(a2 - a1));
    difference() {
        hull() {
            bar_frame(a1, s) socket_body();
            bar_frame(a2, s) socket_body();
            children();
        }
        for (a = [a1, a2]) bar_frame(a, s) { socket_void(); socket_screws(); }
    }
}
