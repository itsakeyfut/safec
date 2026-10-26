void tick(void);

int main(void) {
    int *m[1];
    int **p = m;
    tick();
    int *q = *p;
    if (q == 0) {
        return 1;
    }
    *q = 1;
    return 0;
}
