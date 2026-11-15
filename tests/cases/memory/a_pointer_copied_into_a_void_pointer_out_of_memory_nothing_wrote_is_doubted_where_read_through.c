// `m` is a block nothing wrote, so `memcpy(&t, m, 8)` copies bytes nothing
// wrote into `t`, reported where `t` is read (`SC0411`). A `void *` local
// bounds no width by its type, so what `t` is believed to point at is what
// the source's memory recorded, which is nothing: four bytes read through
// `w` are doubted as well (`SC0412`), as they were before the copy asked
// what a load asks.
//
// Mutation: bound a copy into a `void *` local by nothing in the copy arm of
// `memory/call.rs`. The `SC0412` at `*w` goes.
void *malloc(int n);
void *memcpy(void *d, void *s, int n);

int f(void) {
    int **m = malloc(8);
    if (!m) return 0;
    void *t;
    memcpy(&t, m, 8);
    int *w = t;
    if (!w) return 0;
    return *w;
}
