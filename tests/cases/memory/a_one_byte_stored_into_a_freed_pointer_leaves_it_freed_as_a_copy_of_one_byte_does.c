// One byte stored into a pointer through a `char *` leaves the rest of
// it, so `p` freed is still freed at the `return`: `SC0406`, as after
// `memcpy(c, &z, 1)`. The store replaced the pointer whole, freed state
// and all, and built silent; found by the architecture review of
// 2026-11-13, on `main` too.
void *malloc(int n); void free(void *p);
int *f(void) {
    int *p = malloc(4); if (!p) return 0;
    free(p);
    void *v = &p; char *c = v;
    *c = 1;
    return p;
}
