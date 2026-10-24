void *malloc(int n);
void release_all(void);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    if (q == 0) {
        return 0;
    }
    int **box = malloc(8);
    if (box == 0) {
        return 0;
    }
    release_all();
    *box = q;
    int *r = *box;
    if (r == 0) {
        return 0;
    }
    return *r;
}
