// Apex bracket for the triangle stand.
// Joins the triangle's two legs, and holds the cross bar that runs square to the
// triangle, over to the other triangle's apex bracket. The cross bar carries the
// camera, the Pi and the light.
// The same part turned around serves the other triangle: print 2.
//
// Hardware: 12x M5x10 button head + 12x M5 T-nuts (slot 6), 2 per side per profile.
// The cross bar's screws go through its top and bottom faces.
// Print lying on the flat side away from the cross bar, so its socket points up;
// PETG, 4+ walls, 40% infill, no supports.

include <triangle-2020.scad>

flare = 6;   // how far the cross bar's foot spreads past the socket, each side

// Cross bar leaves the front face (+Y), level with the corner.
module cross_frame() translate([0, outer / 2, 0]) rotate([90, 0, 0]) children();

module foot(y, t) translate([-outer / 2 - flare, y, -outer / 2]) cube([outer + 2 * flare, t, outer]);

difference() {
    union() {
        // Legs run down to either end of the base.
        vee(-leg_rise, -(180 - leg_rise)) foot(-outer / 2, outer);
        hull() {
            cross_frame() socket_body();
            foot(outer / 2 - 1, 1);
        }
    }
    cross_frame() { socket_void(); socket_screws(); }
}
