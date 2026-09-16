// Base corner bracket for the triangle stand.
// Joins the base profile, lying on the table, to a leg rising at leg_rise.
// The same part turned over serves the other end of the base: print 4.
//
// Hardware: 8x M5x10 button head + 8x M5 T-nuts (slot 6), 2 per side per profile.
// Print lying on one of the large flat sides, PETG, 4+ walls, 40% infill, no supports.

include <triangle-2020.scad>

// Base runs along +X, the leg up and across it toward +X.
vee(0, leg_rise);
