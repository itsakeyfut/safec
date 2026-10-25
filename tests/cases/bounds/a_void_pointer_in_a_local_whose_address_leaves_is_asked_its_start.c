void *malloc(int n); void take(void **s);
int main(void){ int *p = malloc(4); if (p == 0) { return 1; } void *v = p + 5; take(&v); return 0; }
