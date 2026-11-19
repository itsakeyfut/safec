void fill(int ***p);

int f(int c) {
    int *m[1];
    int **h = m;
    int ***p = 0;
    if (c) {
        p = &h;
    }
    fill(p);
    int *q = m[0];
    return q == 0;
}
