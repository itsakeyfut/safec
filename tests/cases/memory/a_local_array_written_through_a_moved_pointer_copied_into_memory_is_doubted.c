void *memcpy(void *d, void *s, int n);

int main(void) {
    int x = 0;
    int *m[2];
    int **box[1];
    int **e = m + 1;
    memcpy(box, &e, 8);
    int **f = box[0];
    *f = &x;
    int *r = m[0];
    return r == 0;
}
