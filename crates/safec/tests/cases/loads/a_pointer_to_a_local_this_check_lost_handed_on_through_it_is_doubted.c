int deref(int *p);
void *malloc(int n);
void free(void *p);

int main(void) {
    int *slot = 0;
    int **t2 = &slot;
    int k = 0;
    while (k < 2) {
        int *p = malloc(4);
        if (p == 0) {
            return 0;
        }
        p[0] = 1;
        if (k == 1) {
            return deref(*t2);
        }
        slot = p;
        free(p);
        k = k + 1;
    }
    return 0;
}
