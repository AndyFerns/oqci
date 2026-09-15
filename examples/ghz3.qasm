// A three-qubit GHZ state, with a redundant gate pair for the optimizer to
// find:
//
//   oqci optimize examples/ghz3.qasm --diff
//
OPENQASM 3.0;
include "stdgates.inc";

qubit[3] q;
bit[3] c;

h q[0];

// x;x is the identity — gate-cancellation removes both, even though the
// gate on q[1] sits between them in program order.
x q[2];
h q[1];
x q[2];

cx q[0], q[1];
cx q[1], q[2];

c = measure q;
