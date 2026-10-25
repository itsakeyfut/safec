void *aligned_alloc(int a, int n);

int main(void) {
    int *p = aligned_alloc(16, 1);
    if (p == 0) {
        return 1;
    }
    *p = 5;
    return 0;
}
