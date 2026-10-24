void *malloc(int n);
void free(void *p);
void refill(int **slot);
void release(int *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    p[0] = 1;
    r[0] = 2;
    *tab = p;
    free(p);
    release(*tab);
    return 0;
}
