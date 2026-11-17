void *malloc(int n);
void *memcpy(void *d, void *s, int n);

int release_all(void);

int f(void **out_) {
    int x;
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **box_ = malloc(8);
    if (box_ == 0) {
        return 0;
    }
    *box_ = a;
    a[0] = 1;
    r = (x = a[0]) + (memcpy(out_, box_, 8) != 0) + release_all();
    return r;
}
