int main(int c) {
    int x = 1;
    int *m[2];
    int **box[1];
    box[0] = m + 1;
    int **f = box[0];
    *f = &x;
    int **e = m + 2;
    while (c) {
        e = e - 1;
        c = c - 1;
    }
    int *q = *e;
    return q == 0;
}
