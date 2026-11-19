void *malloc(int n);
void tick(void);

int keep(int ****box) {
    int *m[1];
    int ***inner = malloc(8);
    if (box == 0 || inner == 0) {
        return 1;
    }
    *inner = m;
    *box = inner;
    tick();
    int *q = m[0];
    return q == 0;
}
