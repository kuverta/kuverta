// The Pi and camera mount from pi-camera-bracket.scad, for a Raspberry Pi 4.
// Everything but the plate on top is the same part: the camera slot, the clamp to
// the bar's side slot, the screws.
//
// The Pi 4 is 85 x 56, far wider than the bar, so its plate reaches out over the
// open side of the C. It stops flush with the back face, so the part still stands on
// that face to print. The Pi then sits off to that side of the bar, above the camera,
// where it cannot be in the picture.
//
// The camera plugs into the Pi 4's CSI connector with the standard 15-way ribbon,
// not the narrow Pi Zero one.
//
// Hardware: as pi-camera-bracket.scad, with 4x M3x8 for the Pi's taller posts.
// Print standing on its back, the face with the two M5 holes; PETG, 4+ walls,
// 40% infill, no supports.

include <pi-camera-bracket.scad>

// Pi 4: 85 x 56 board, holes 58 x 49 apart, 3.5 mm in from the edges.
pi_board  = [56, 85];
pi_holes  = [49, 58];
pi_x      = back - 3.5 - pi_holes[0] / 2;   // the board's edge flush with the back face
post_h    = 5;                              // air under a board that gets warm, and room
                                            // for what is soldered on its underside
plate     = [[back - pi_board[0], back], [-35, 35]];
