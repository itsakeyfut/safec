int main(void) {
    int x = 1;
    int *m[2];
    int **box[1];
    box[0] = m + 1;
    int **f = box[0] - 1;
    *f = &x;
    int *r = m[1];
    return r == 0;
}
