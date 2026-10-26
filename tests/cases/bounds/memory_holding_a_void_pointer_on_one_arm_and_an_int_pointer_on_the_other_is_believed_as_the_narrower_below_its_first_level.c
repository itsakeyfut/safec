void *malloc(int n);
void free(void *p);
void run(void ***v, int ***i, int c) {
    void **box = malloc(8);
    if (box == 0) { return; }
    void *pv = v;
    void *pi = i;
    if (c) {
        *box = pv;
    } else {
        *box = pi;
    }
    void *pb = box;
    int ****pp = pb;
    int ***x = *pp;
    if (x != 0) {
        int **y = *x;
        if (y != 0) {
            int *d = *y;
            if (d != 0) {
                *d = 1;
            }
        }
    }
    free(box);
}
