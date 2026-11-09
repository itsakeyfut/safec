void *malloc(int n);
void free(void *p);
int *g(int * _Nonnull x) {
  int **b = 0;
  for (int j = 0; j < 2; j++) {
    if (b) *b = x;
    for (int i = 0; i < 2; i++) {
      free(b);
      b = malloc(16);
    }
  }
  int *y = 0;
  if (b) y = *b;
  free(b);
  return y;
}
