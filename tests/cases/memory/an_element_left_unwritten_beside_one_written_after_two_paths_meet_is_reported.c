int main(int c) {
    int x = 1;
    int *m[2];
    int **e = m + 1;
    if (c) {
        x = 2;
    }
    *e = &x;
    int *q = m[0];
    return q == 0;
}
