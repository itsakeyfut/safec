void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int main(void) {
    char c = 0;
    void *vc = &c;
    int *bad = vc;
    int **slot = malloc(8);
    if (slot == 0) { return 1; }
    memcpy(slot, &bad, 8);
    int *q = *slot;
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
