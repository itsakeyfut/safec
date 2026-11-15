// `s` is `&a` on one path and a pointer read out of the caller's memory on
// the other, so the copy may read `a`, which nothing wrote, or whatever the
// caller stored. `r` may be indeterminate, not certainly: the report says
// "may be read", and does not claim the copy read `a` (#946).
void *memcpy(void *d, void *s, int n);

int *f(int c, int ***box) {
    int *a;
    int **s;
    if (c) s = &a; else s = *box;
    int *r;
    memcpy(&r, s, 8);
    return r;
}
