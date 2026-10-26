int main(void) {
    int x = 0;
    int *m[2];
    int **e = m + 2;
    while (e != m) {
        e = e - 1;
        *e = &x;
    }
    int *q = m[0];
    return q == &x;
}
