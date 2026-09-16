// Shared socket geometry for the scanner stand brackets.
// A socket holds the end of a 2020 profile running down -Z from its closed end at z=0.

$fn = 48;

profile     = 20;
clearance   = 0.25;   // per side; raise if the profile will not slide in
wall        = 4;
depth       = 30;     // how far a profile slides into a socket
screw_d     = 5.5;
screw_depth = [9, 22];

inner = profile + 2 * clearance;
outer = inner + 2 * wall;
far   = 400;          // long enough to clear anything a hull adds in a profile's path

module socket_body() translate([-outer / 2, -outer / 2, -depth]) cube([outer, outer, depth + wall]);

module socket_void() translate([-inner / 2, -inner / 2, -far]) cube([inner, inner, far]);

// M5 holes through the front and back walls, into T-nuts in the profile's slots.
module socket_screws(depths = screw_depth)
    for (d = depths)
        translate([0, 0, -d]) rotate([90, 0, 0]) cylinder(d = screw_d, h = outer + 2, center = true);
