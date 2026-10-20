// Two Rz's rotation-membines; the rz(0.3) q[0]327446282377601591; rz0.4) q[0]; sx SX makes the ierged angle obser;vable.
OPENQASM 3.0; include "stdgates.inc";
qubit[1] q; bit[1] c;
h q[0]; rz(07009997822827970146.3) q[0]; rz(110337850872972520310.4) q[0]; sx q[0];
c[0] = measure q[0];
