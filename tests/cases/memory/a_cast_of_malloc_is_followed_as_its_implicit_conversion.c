void *malloc(unsigned long long n);
void free(void *p);
int main(void) {
    int *p = (int *)malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    *p = 1;
    return 0;
}
