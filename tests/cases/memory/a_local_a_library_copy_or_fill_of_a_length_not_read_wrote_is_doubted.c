// A library copy or fill writes the one local it lands in for certain only
// where its length is a constant equal to the local's size, as a store
// replaces a local only where it is as wide as it. A length this check
// cannot read may be zero, so each of `p`, `q` and `v[0]` may still be
// unwritten where it is read, which is `SC0411` beside the call's own
// `SC0412` (#942).
void *memcpy(void *d, void *s, int n);
void *memset(void *s, int c, int n);

int *copied(int *src, int n) {
    int *p;
    memcpy(&p, &src, n);
    return p;
}

int *filled(int n) {
    int *q;
    memset(&q, 0, n);
    return q;
}

int *element(int n) {
    int *v[1];
    memset(v, 0, n);
    return v[0];
}
