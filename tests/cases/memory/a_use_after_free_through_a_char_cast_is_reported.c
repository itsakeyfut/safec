void *malloc(unsigned long long n);
void free(void *p);
int main(void) {
    int *p = malloc(8);
    if (!p) {
        return 0;
    }
    char *c = (char *)p;
    free(p);
    c[0] = 1;
    return 0;
}
