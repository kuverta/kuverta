// The Pi 4 mount from pi4-camera-bracket.scad, closed into a ring round the bar.
// Instead of clamping onto the bar's side it goes on over the end of the bar, before
// the apex brackets are fitted, and slides along it to the middle. The bar runs
// through a square hole with 0.25 mm clearance all round.
//
// The ribbon cable leaves the camera slot through its mouth, along the bar, and
// comes up to the Pi past the end of the ring.
//
// Hardware: 4x M3x8 self-tapping for the Pi. The two M5 holes stay: with an M5x10
// and a T-nut in the bar's side slot they stop the ring sliding once it is placed.
// Print standing on its back, the face with the two M5 holes; PETG, 4+ walls,
// 40% infill. The closing wall bridges 20 mm over the bar's tunnel, which PETG
// manages without supports.

include <pi4-camera-bracket.scad>

closed    = true;
