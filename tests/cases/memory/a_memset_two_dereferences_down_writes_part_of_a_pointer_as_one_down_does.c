// A `memset` of four bytes through `*box`, into memory later let leave as
// pointers, writes part of a pointer there, as `memset(t, 1, 4)` through a
// local does and as a narrow store at any depth does: `SC0413` where the
// block leaves. Asked of a destination with no projection alone, it noted
// nothing; found by the architecture review of 2026-11-13.
void *malloc(int n);
void *memset(void *d, int c, int n);
void use(void);
void f(void ***pp) {
    if (!pp) return;
    void *m = malloc(16);
    if (!m) return;
    void **box = malloc(8);
    if (!box) return;
    *box = m;
    if (!*box) return;
    memset(*box, 1, 4);
    void **q = m;
    *pp = q;
    use();
}
