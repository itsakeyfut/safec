int main(void) {
    int x = 1;
    int *m[2];
    int **box[1];
    box[0] = m + 1;
    int **f = box[0];
    *f = &x;
    m[1] = &x;
    int *r = m[0];
    return r == 0;
}
