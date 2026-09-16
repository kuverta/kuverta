// Angled T-bracket for the scanner stand.
// Sits on the base 2020 profile lying on the table and holds the post so it rises
// at post_rise above the base, leaning sideways across it (90 degrees to the base
// when seen from above).
// Open underneath, so the base profile still lies flat on the table.
//
// Hardware: 8x M5x10 button head + 8x M5 T-nuts (slot 6), 2 per side per profile.
// Print lying on the saddle side wall the post leans away from (-Y), so the post
// socket points up at 45 degrees; PETG, 4+ walls, 40% infill, no supports.

include <common-2020.scad>

post_rise    = 45;          // angle of the post above the base
post_offset  = [0, 0, 12];  // where the post's end sits above the base's top face
saddle       = [-30, 30];   // saddle extent along the base
saddle_screws = [-23, 23];  // where the base is screwed down, clear of the post
table_gap    = 1.5;         // side walls stop this far above the table

// Base profile runs along X, top face at z=0, bottom face on the table at z=-profile.
module saddle_body(from = saddle[0], to = saddle[1])
    translate([from, -outer / 2, -profile + table_gap])
        cube([to - from, outer, profile - table_gap + clearance + wall]);

module base_void() translate([-far, -inner / 2, -far]) cube([2 * far, inner, far + clearance]);

module saddle_screws()
    for (x = saddle_screws)
        translate([x, 0, -profile / 2]) rotate([90, 0, 0]) cylinder(d = screw_d, h = outer + 2, center = true);

// Post socket, tilted so its -Z points up and across the base toward +Y.
// Its screws then run along X, through the faces left clear beside the saddle.
module post_frame() translate(post_offset) rotate([0, 0, 90]) rotate([0, -(90 + post_rise), 0]) children();

difference() {
    union() {
        saddle_body();
        hull() {
            saddle_body(-outer / 2, outer / 2);
            post_frame() socket_body();
        }
    }
    base_void();
    saddle_screws();
    post_frame() { socket_void(); socket_screws(); }
}
