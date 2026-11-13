int main(void) {
    int x = 1;
    int s = 0;
    int **box[1];
    for (int i = 0; i < 2; i++) {
        int *m[2];
        if (i == 0) {
            box[0] = m + 1;
        } else {
            box[0] = m;
            int **f = box[0];
            *f = &x;
            int *r = m[0];
            s = s + (r != 0);
        }
    }
    return s;
}
