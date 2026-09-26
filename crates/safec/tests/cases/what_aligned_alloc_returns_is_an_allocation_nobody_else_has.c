void *aligned_alloc(int alignment, int size);
void free(void *p);
int abs(int x);

int main(void) {
    int *q = aligned_alloc(4, 4);
    if (q == 0) {
        return 0;
    }
    int a = abs(-1);
    *q = a;
    free(q);
    return 0;
}
