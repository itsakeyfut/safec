int main(void) {
    int x = 1;
    int *m[2];
    int **e = m + 1;
    int ***pe = &e;
    **pe = &x;
    int *r = m[0];
    return r == 0;
}
