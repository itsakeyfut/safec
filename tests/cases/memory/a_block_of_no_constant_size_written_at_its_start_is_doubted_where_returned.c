// No store is known to be as wide as a block whose size is not a constant,
// so `*b = x;` may have missed the rest of it (#932).
void *malloc(int n);
int **f(int * _Nonnull x, int n) { int **b = malloc(n); if (!b) return 0; *b = x; return b; }
