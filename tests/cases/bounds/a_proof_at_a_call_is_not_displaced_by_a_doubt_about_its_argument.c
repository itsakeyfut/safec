void *malloc(int n);
void *memset(void *d, int c, int n);
int main(int argc) {
    int *p = 0;
    if (argc) { p = malloc(4); } else { p = malloc(2); }
    if (p == 0) { return 1; }
    memset(p, *p, 8);
    return 0;
}
