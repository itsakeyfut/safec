// A `char` stored two dereferences down, into memory of pointers, writes
// part of the pointer there, as `*c = 1;` does one down: `SC0413` at the
// store. Asked at one dereference alone, it wrote the slot whole; found
// by the architecture review of 2026-11-13.
void *malloc(int n);
int f(int **out, int *x) {
    if (!out) return 1;
    int **m = malloc(8);
    if (!m) return 1;
    *m = x;
    void *v = m;
    char *c = v;
    char **cc = malloc(8);
    if (!cc) return 1;
    *cc = c;
    **cc = 1;
    *out = *m;
    return 0;
}
