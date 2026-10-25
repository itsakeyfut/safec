void *malloc(int n);
void h(int **pp);
int main(void) { int *a = malloc(4); if (a == 0) { return 1; } int *q = 0; int **pp = &q; q = a + 5; h(pp); return 0; }
