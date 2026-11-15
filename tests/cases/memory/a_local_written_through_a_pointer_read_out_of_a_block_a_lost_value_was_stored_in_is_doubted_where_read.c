// `v` is moved past its end and back, which this check reads as a value it
// stopped following, so a pointer read back out of the block it was stored
// in may be that lost value (`Why::LostRead`), and the write `*q = z` is
// not believed to land in `x` alone: `x` is doubted where it is read (#946).
//
// **A doubt this program does not need.** `&v + 1` is one past `v` (C17
// 6.5.6 p8), so `w[-1]` is `v`, `q` is `&x`, and `*q = z` writes `x`. The
// mark is kept because a value read back where a lost one was stored may
// be anything, and dropping it here only trades this `SC0411` for an
// `SC0406` as unneeded, measured.
void *malloc(int n);

int *f(int k, int *z) {
    int *x;
    int **v = 0;
    int ***w = &v + 1;
    w[-1] = &x;
    int ***b = malloc(8);
    if (!b) return z;
    *b = v;
    int **q = *b;
    *q = z;
    return x;
}
