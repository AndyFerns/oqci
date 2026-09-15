// A parameterized (VQE-style) circuit. Its rotations stay symbolic until you
// supply values, so QIR emission is deliberately unavailable until then:
//
//   oqci compile examples/parameterized.qasm
//   oqci compile examples/parameterized.qasm --bind theta=1.5708 --bind phi=0.7854
//
OPENQASM 3.0;
include "stdgates.inc";

input float[64] theta;
input float[64] phi;

qubit[2] q;
bit[2] c;

ry(theta) q[0];
cx q[0], q[1];
rz(phi) q[1];

// Two adjacent same-axis rotations with concrete angles: rotation-merge
// folds these into a single rz(pi/2).
rz(pi/4) q[0];
rz(pi/4) q[0];

c = measure q;
