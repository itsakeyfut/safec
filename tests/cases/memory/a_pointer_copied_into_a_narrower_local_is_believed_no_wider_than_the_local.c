// `memcpy(&s, m, 8)` puts in `s` the bytes of a pointer `m` says points at
// an `int`, but `s` is a `char *`, and a copy is believed no wider than the
// local it lands in, as a load at that local's type would be: so `*w`,
// four bytes through what arrived as a `char *`, is a doubt (`SC0412`).
// `t` turns the `char *` into an `int *` without a cast, which this compiler
// does not read yet; the two `SC0403`s are the null doubts of `m` and `w`.
//
// Mutation: drop the bound by the local's own type in the copy arm of
// `memory/call.rs`. The `SC0412` at `*w` goes.
void *memcpy(void *d, void *s, int n);

int f(int **m) {
    char *s;
    memcpy(&s, m, 8);
    void *t = s;
    int *w = t;
    return *w;
}
