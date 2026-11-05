void mod(int ***p);
int main(int c) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    mod(&e);
    int ***pe = &e;
    int **f = *pe;
    *f = &x;
    int *r = m[1];
    return r == 0;
}
