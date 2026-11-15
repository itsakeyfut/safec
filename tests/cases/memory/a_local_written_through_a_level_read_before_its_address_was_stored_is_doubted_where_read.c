// `pp` is read out of the caller's memory before `&b` is stored there, and
// `**ppp = m` writes through it. A level read out of memory is asked what
// code this check cannot read may name at the write, which by then is `b`,
// so `b` is doubted where it is read, "may be read" (#946).
//
// **Less than this program allows.** `pp` was read before `b`'s address
// was a value anywhere, so it is not `&b`, and `b` is certainly read before
// anything wrote it. Asking the level as of its read, as `named_at_read`
// does for the pointer itself (#648), would say so; asking it as of the
// write is the cautious answer, and it is kept.
int *f(int ***box, int ***box2, int *m) {
    int *b;
    int **pp;
    pp = *box;
    *box2 = &b;
    int ***ppp = &pp;
    **ppp = m;
    return b;
}
