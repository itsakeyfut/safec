int main(void) {
    int x = 0;
    int *m[4];
    int **e = m + 2;
    int **f = e - 1;
    *f = &x;
    int *r = m[3];
    return r == 0;
}
