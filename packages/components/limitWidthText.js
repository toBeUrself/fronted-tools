import { observer } from "mobx-react-lite";
import { useEffect, useRef, useState } from 'react';

const LimitWidthLabel = ({
  maxWidth,
  label,
  className,
}) => {
  const svgRef = useRef(null);
  const [adjust, setAdjust] = useState(false);
  const [textWidth, setTextWidth] = useState('100%');

  useEffect(() => {
    if (svgRef.current) {
      const { width } = svgRef.current.getBBox();
      
      if (width >= Number(maxWidth)) {
        setAdjust(true);
      } else {
        setTextWidth(width);
      }
    }
  }, [maxWidth]);

  return (
    <svg ref={svgRef} className={className} xmlns="http://www.w3.org/2000/svg" version="1.1" width={textWidth} height={'100%'}>
      <text lengthAdjust="spacingAndGlyphs" dominantBaseline="text-before-edge" textLength={adjust ? '100%' : ''} text-anchor="start" x={0} y={0} fill="currentColor">{label}</text>
    </svg>
  );
};

export default observer(LimitWidthLabel);
