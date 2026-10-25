void *malloc(int n);
void keep(int **pp);
void other(void);
void use(int *p);
int main(void) { int *a = malloc(4); if (a == 0) { return 1; } keep(&a); a = a + 5; other(); use(a); return 0; }
