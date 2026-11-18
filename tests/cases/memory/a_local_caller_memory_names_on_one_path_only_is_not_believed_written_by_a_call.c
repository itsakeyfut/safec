void fill(int ***b);
int f(int ***out, int c) {
    int *x;
    if (out == 0) return 0;
    if (c) *out = &x;
    fill(out);
    return x != 0;
}
