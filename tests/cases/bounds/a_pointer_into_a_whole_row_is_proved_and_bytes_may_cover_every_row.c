// A row that is the whole object is no row: a pointer into an array of one
// row keeps its position where it joins a plain pointer, and `p[1]` is
// proved. A library function writing bytes through a pointer that walks a
// row may write the whole object, which C17 6.3.2.3 p7 allows: the
// `memset` is proved too (#947).
void *memset(void *d, int c, int n);

int joined(int c) {
    int b[1][2];
    int x[2];
    b[0][1] = 1;
    x[1] = 2;
    int *p = x;
    if (c) p = b[0];
    return p[1];
}

int filled(void) {
    int a[2][2];
    int *p = a[0];
    memset(p, 0, 16);
    return a[1][1];
}
