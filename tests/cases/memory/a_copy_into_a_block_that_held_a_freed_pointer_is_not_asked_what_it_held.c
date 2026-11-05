void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *m = p;
    free(p);
    int *v = 0;
    memcpy(m, &v, 8);
    int *r = *m;
    return r == 0;
}
