// A copy of the first slot of a local array of two, which a store wrote,
// reads it as `*out = a[0];` reads it: the transfer and the report count
// the slots it reads by one rule (the memory model rebuilt on #924, stage
// 2). Read apart, the transfer asked the array at any element, and `*out`
// was doubted after the copy where the store left it written.
void *memcpy(void *d, void *s, int n);
int f(int ** _Nonnull out, int * _Nonnull x) {
    int *a[2];
    a[0] = x;
    memcpy(out, a, 8);
    int *r = *out;
    return r == x;
}
