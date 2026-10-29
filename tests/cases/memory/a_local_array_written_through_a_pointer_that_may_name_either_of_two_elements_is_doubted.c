int main(int c) {
    int x = 1;
    int *m[2];
    int **e1 = m + 1;
    int **e2 = m;
    int ***pp = &e2;
    if (c) {
        pp = &e1;
    }
    **pp = &x;
    int *r = m[0];
    return r == 0;
}
