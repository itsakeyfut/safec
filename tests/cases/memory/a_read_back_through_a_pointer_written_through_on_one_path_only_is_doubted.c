int main(int c, int d) {
    int x = 1;
    int *m[2];
    int **e = m + 2;
    while (c) {
        e = e - 1;
        c = c - 1;
    }
    if (d) {
        *e = &x;
    }
    int *q = *e;
    return q == 0;
}
