// `*pp = 0` lands either in `L` or in memory the caller holds, a store this
// check cannot place, so `L` is marked as written by code it cannot read
// (`Why::Foreign`) and the write `*L = z` is not believed to land in `x`
// alone: `x` is doubted where it is read (#946).
//
// **A doubt this program does not need.** With `c` false the caller's
// memory cannot be `L`, whose address never left, so `*L = z` writes `x`;
// with `c` true `*L` dereferences a null, which `SC0403` reports. The mark
// is kept because a pointer some unplaced write may have replaced may name
// what it does not say, and dropping it here only trades this `SC0411`
// for an `SC0406` as unneeded, measured.
int *f(int c, int ****box, int *z) {
    int *x;
    int **L = &x;
    int ***pp;
    if (c) pp = &L; else pp = *box;
    *pp = 0;
    *L = z;
    return x;
}
