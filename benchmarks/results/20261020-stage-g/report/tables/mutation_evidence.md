<!-- generated from serialized data; do not edit -->
**Mutation evidence: which checker caught each injected defect**

| mutant | caught_by | pass-equivalence | lowering-equivalence | decomposition | in-compiler-verifier | qir-structure | unit-tests |
|---|---|---|---|---|---|---|---|
| (unmutated baseline) | SURVIVED | – | – | – | – | – | – |
| rotation-merge-sign-flip | pass-equivalence,in-compiler-verifier,unit-tests | caught | – | – | caught | – | caught |
| cancel-non-inverse-pair | in-compiler-verifier | – | – | – | caught | – | – |
| cancel-across-measurement | unit-tests | – | – | – | – | – | caught |
| canonicalize-commutes-h-x | SURVIVED | – | – | – | – | – | – |
| fusion-wrong-euler-angle | pass-equivalence | caught | – | – | – | – | – |
| h-to-rz-sx-drops-a-step | lowering-equivalence,decomposition,in-compiler-verifier,unit-tests | – | caught | caught | caught | – | caught |
| rx-to-rz-sx-wrong-angle | lowering-equivalence,decomposition | – | caught | caught | – | – | – |
| swap-to-cx-wrong-orientation | lowering-equivalence,decomposition,in-compiler-verifier | – | caught | caught | caught | – | – |
| routing-forged-final-layout | lowering-equivalence | – | caught | – | – | – | – |
| qir-cx-operands-swapped | SURVIVED | – | – | – | – | – | – |
