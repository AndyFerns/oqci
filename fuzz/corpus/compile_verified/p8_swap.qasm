// A program-level SWAP, decomposed by swap-to-cx: the 1 must move to q1.
OPENQASM 3.0; include "stdgates.inc";
qubit[2] q; bit[2] c;
x q[0]; swap q[0], q[1];
c = measure q;
