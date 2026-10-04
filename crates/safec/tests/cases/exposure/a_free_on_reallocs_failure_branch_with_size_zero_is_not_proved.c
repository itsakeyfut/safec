void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    int *q = realloc(p, 0);
    if (q == 0) {
        free(p);
        return 0;
    }
    q[0] = 2;
    free(q);
    return 0;
}
