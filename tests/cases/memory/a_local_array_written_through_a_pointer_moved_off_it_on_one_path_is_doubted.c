int main(int c, int i) {
    int x = 1;
    int *m[2];
    int **e;
    if (c) {
        e = m + 1;
    } else {
        e = m + i;
    }
    *e = &x;
    int *r = m[0];
    return r == 0;
}
