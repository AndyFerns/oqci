// A Bell state: the canonical two-qubit entangled pair.
//
//   oqci compile examples/bell.qasm
//
OPENQASM 3.0;
include "stdgates.inc";

qubit[2] q;
bit[2] c;

h q[0];
cx q[0], q[1];
c = measure q;
