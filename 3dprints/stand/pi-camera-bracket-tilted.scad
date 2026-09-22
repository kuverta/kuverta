// The Pi Zero mount from pi-camera-bracket.scad, with the Pi leaned up at pi_tilt.
// The plate turns about its edge along the back face, so the side over the open
// side of the C rises. What sits on the bar stays flat, with a wedge filling in
// between it and the leaned plate.
//
// Hardware: as pi-camera-bracket.scad.
// Print standing on its back, the face with the two M5 holes; PETG, 4+ walls,
// 40% infill, no supports. The leaned plate is 30 degrees off upright that way,
// which prints without help.

include <pi-camera-bracket.scad>

pi_tilt   = 30;

// Leaning up tips the post tops toward the back face, so the Pi moves in far enough
// that they stay behind it and the part still stands flat on that face to print.
pi_x      = back - post_d / 2 - pi_holes[0] / 2 - post_h * tan(pi_tilt) - 0.5;

// The plate grows on the far side by as much, so the far posts stay on it.
plate     = [[pi_x - pi_holes[0] / 2 - 3.5, back], [-34, 34]];
