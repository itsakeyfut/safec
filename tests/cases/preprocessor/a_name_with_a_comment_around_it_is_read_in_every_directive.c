#/**/define/**/A/**/1
#define F(/**/a/**/)/**/a
#undef/**/A/**/
#define A 2
#ifdef/**/A/**/
int a = F(A);
#endif
#ifndef/**/B/**/
int b = 1;
#endif
#pragma/**/STDC/**/FP_CONTRACT ON
#line/**/40
int main(void) { return a + b; }
