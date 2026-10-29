int main(int c) {
    int x = 1;
    int *m[2];
    int **box[1];
    box[0] = m;
    if (c) {
        box[0] = m + 1;
    }
    int **f = box[0];
    *f = &x;
    int *r = m[0];
    return r == 0;
}
