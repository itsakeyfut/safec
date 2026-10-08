void *malloc(int n);
void release(void);
int **get_slot(void);
int f(int c) {
    int **pp;
    if (c) {
        pp = get_slot();
    } else {
        pp = malloc(8);
    }
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    if (q == 0) {
        return 0;
    }
    release();
    return *q;
}
