void *malloc(int n);
void *memset(void *d, int c, int n);

int main(void) {
    char *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, 8);
    return 0;
}
