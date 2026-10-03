void free(void *p);
int f(int *p) {
    free(p);
    free(p);
    return 0;
}
int main(void) { return f(0); }
