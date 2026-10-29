int main(void) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    int ***pe = &e;
    int **f = *pe - 1;
    *f = &x;
    int *r = m[1];
    return r == 0;
}
