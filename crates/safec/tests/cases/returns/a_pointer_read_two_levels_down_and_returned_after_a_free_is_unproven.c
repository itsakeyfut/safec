void *malloc(int n);
void free(void *p);

int *get(void) {
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
    int **t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    *t2 = p;
    *t3 = t2;
    free(p);
    return **t3;
}
