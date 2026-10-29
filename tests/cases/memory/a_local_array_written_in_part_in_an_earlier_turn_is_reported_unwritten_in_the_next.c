int main(int c) {
    int x = 1;
    int s = 0;
    for (int i = 0; i < 2; i++) {
        int *m[2];
        int *r = m[0];
        s = s + (r != 0);
        int **box[1];
        box[0] = m + 1;
        int **f = box[0];
        *f = &x;
    }
    return s;
}
