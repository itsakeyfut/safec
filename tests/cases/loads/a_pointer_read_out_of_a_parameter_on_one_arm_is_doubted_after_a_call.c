void *malloc(int n);
int cond(void);
void release_all(void);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int *q = malloc(4);
    if (q == 0) {
        return 0;
    }
    q[0] = 1;
    if (cond()) {
        q = *pp;
    }
    if (q == 0) {
        return 0;
    }
    release_all();
    return *q;
}
